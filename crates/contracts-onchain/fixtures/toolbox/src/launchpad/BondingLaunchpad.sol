// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "openzeppelin/token/ERC20/IERC20.sol";
import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";
import {SimpleBrake} from "src/system/SimpleBrake.sol";
import {SafeToken} from "src/common/SafeToken.sol";
import {FixedSupplyToken} from "src/token/FixedSupplyToken.sol";
import {AmmFactory} from "src/amm/AmmFactory.sol";
import {AmmPair} from "src/amm/AmmPair.sol";

/// @title 예제 5 — 본딩 커브 런치패드 (안티스나이핑 + AMM 졸업)
/// @notice 새 토큰을 상수곱 가상 리저브 커브로 판매하고, 모금 목표에 도달하면
///         예제 4의 AMM 페어로 유동성을 이전(졸업)한다. 졸업 후 커브는 폐쇄.
///
///         법적 고지: 토큰 발행은 앱 레지스트리 제한 카테고리다.
///         공개 배포 전 법률 검토를 마쳐야 한다 (examples/launchpad/README.md).
/// @dev
///  커브 수학 — 가상 리저브:
///   vq = curve 보유 quote + quoteFloor,  vt = curve 보유 토큰 + tokenFloor
///   매수 out = vt * netIn / (vq + netIn)
///   매도 out = vq * in   / (vt + in)
///   fee(매매 수수료)와 스나이프 세금은 커브 밖(treasury)으로 나가므로
///   유효 리저브 곱 vq*vt는 매매로 줄지 않는다(나눗셈 버림분만큼 증가).
///   이 불변식을 fuzz 테스트로 매 거래마다 검증한다.
///
///  안티스나이핑 — 두 겹:
///   1. 감쇠 세금: launch 후 snipeWindow 초 동안 매수액에서 snipeTaxBps가
///      선형 감쇠하며 treasury로 나간다 (t=0 최대, t=window 이후 0).
///   2. 개인 한도: 졸업 전 보유(cap) 상한. 커브 토큰 잔액 조회로 검사해
///      추가 상태 비용이 없다.
///
///  졸업:
///   curve 보유 quote >= graduationTarget이면 누구나 graduate()를 호출할 수
///   있다. 전액 + 남은 토큰을 새 AMM 페어로 옮기고 LP를 0x..dEaD에 영구
///   잠근다. 프론트런해도 이득이 없는 구조(무차별 public, LP 회수 불가).
///   누군가 미리 (token, quote) 페어를 만들어 둔 경우 LP 공급이
///   MINIMUM_LIQUIDITY 이하일 때만 재사용한다 — 이상이면 PairPoisoned.
///
///  F 매핑 (docs/safety-checklist.md):
///   F-01: 모든 진입점 nonReentrant + SafeToken 경로.
///   F-02: 유입 quote/토큰 전부 pull의 도착량(delivered) 기준 정산.
///   F-03: quote/팩토리 무코드 거부(생성자). 토큰 이동은 SafeToken.
///   F-04: 뷰는 상수 시간 — 순회 없음.
contract BondingLaunchpad is SimpleBrake, ReentrancyGuard {
    /// @dev AmmPair.MINIMUM_LIQUIDITY(=1000)와 수동 동기화 — 졸업 페어
    ///      재사용 조건에 쓴다. AmmPair 쪽을 바꾸면 여기도 바꿀 것.
    uint256 private constant _MIN_LIQ = 1000;
    uint256 private constant _BPS_MAX = 10_000;
    /// @dev AMM 예제와 같은 영구 잠금 주소 (OZ v5는 address(0) mint 금지).
    address private constant _DEAD = 0x000000000000000000000000000000000000dEaD;

    /// @param name          커브 토큰 이름 (FixedSupplyToken으로 1회 발행)
    /// @param symbol        커브 토큰 심볼
    /// @param tokenSupply   커브 판매 총량 (전량을 커브가 보유하고 시작)
    /// @param quoteFloor    가상 quote 리저브 오프셋 (초기 가격 결정)
    /// @param tokenFloor    가상 토큰 리저브 오프셋
    /// @param graduationTarget 졸업 목표 — curve 보유 quote 도달 시 graduate 가능
    /// @param feeBps        매수·매도 공통 수수료 (basis points, treasury로)
    /// @param snipeTaxBps   launch 직후 매수 세금 최대치 (감쇠)
    /// @param snipeWindow   스나이프 세금 감쇠 시간 (초). snipeTaxBps > 0이면 > 0
    /// @param perBuyerCap   졸업 전 개인 보유 상한. 0이면 제한 없음
    /// @param treasury      수수료·스나이프 세금 수령인
    struct Config {
        string name;
        string symbol;
        uint256 tokenSupply;
        uint256 quoteFloor;
        uint256 tokenFloor;
        uint256 graduationTarget;
        uint256 feeBps;
        uint256 snipeTaxBps;
        uint256 snipeWindow;
        uint256 perBuyerCap;
        address treasury;
    }

    error Closed();
    error TargetReached();
    error AlreadyGraduated();
    error NotGraduable(uint256 raised, uint256 target);
    error PairPoisoned(address pair);
    error InsufficientInput();
    error InsufficientOutput(uint256 wanted, uint256 got);
    error ExcessivePurchase(uint256 wouldHold, uint256 cap);
    error InvalidConfig();

    event Bought(address indexed buyer, uint256 quoteNetIn, uint256 tokensOut, uint256 fee, uint256 snipeTax);
    event Sold(address indexed seller, uint256 tokensIn, uint256 quoteOut, uint256 fee);
    event Graduated(address indexed pair, uint256 quoteSeeded, uint256 tokenSeeded, uint256 lpLocked);

    IERC20 public immutable quote;
    AmmFactory public immutable factory;
    FixedSupplyToken public immutable curveToken;
    address public immutable treasury;
    uint256 public immutable quoteFloor;
    uint256 public immutable tokenFloor;
    uint256 public immutable graduationTarget;
    uint256 public immutable feeBps;
    uint256 public immutable snipeTaxBps;
    uint256 public immutable snipeWindow;
    uint256 public immutable perBuyerCap;
    uint256 public immutable launchTime;

    /// @dev bool + address는 한 슬롯으로 팩된다 — 졸업이 유일한 저장 시점.
    bool public graduated;
    address public graduatePair;

    modifier notClosed() {
        if (graduated) revert Closed();
        _;
    }

    constructor(address guardian, IERC20 quote_, AmmFactory factory_, Config memory cfg) SimpleBrake(guardian) {
        // F-03: 무코드 주소는 배포 단계에서 거부한다.
        if (address(quote_).code.length == 0 || address(factory_).code.length == 0) {
            revert InvalidConfig();
        }
        if (cfg.treasury == address(0)) revert InvalidConfig();
        if (cfg.tokenSupply == 0 || cfg.quoteFloor == 0 || cfg.tokenFloor == 0) revert InvalidConfig();
        if (cfg.graduationTarget == 0) revert InvalidConfig();
        if (cfg.feeBps + cfg.snipeTaxBps > _BPS_MAX) revert InvalidConfig();
        if (cfg.snipeTaxBps > 0 && cfg.snipeWindow == 0) revert InvalidConfig();

        quote = quote_;
        factory = factory_;
        treasury = cfg.treasury;
        quoteFloor = cfg.quoteFloor;
        tokenFloor = cfg.tokenFloor;
        graduationTarget = cfg.graduationTarget;
        feeBps = cfg.feeBps;
        snipeTaxBps = cfg.snipeTaxBps;
        snipeWindow = cfg.snipeWindow;
        perBuyerCap = cfg.perBuyerCap;
        launchTime = block.timestamp;

        curveToken = new FixedSupplyToken(cfg.name, cfg.symbol, cfg.tokenSupply, address(this));
        if (address(curveToken) == address(quote_)) revert InvalidConfig();
    }

    // ---------------------------------------------------------------- 커브 매매

    /// @notice quote로 커브 토큰을 산다. 수수료와 스나이프 세금을 제외한
    ///         몫만 가격 산정에 들어간다.
    /// @param quoteIn      지불 quote (pull — 도착량 기준 정산, F-02)
    /// @param minTokensOut 슬리피지 하한
    function buy(uint256 quoteIn, uint256 minTokensOut) external nonReentrant whenEntryOpen notClosed {
        if (quote.balanceOf(address(this)) >= graduationTarget) revert TargetReached();
        uint256 preQuote = quote.balanceOf(address(this));
        uint256 delivered = SafeToken.pull(quote, msg.sender, quoteIn);
        if (delivered == 0) revert InsufficientInput();

        (uint256 fee, uint256 snipeTax, uint256 netIn) = _buyTaxes(delivered);
        SafeToken.push(quote, treasury, fee + snipeTax);

        uint256 out = _buyTokenOut(netIn, preQuote);
        if (out == 0) revert InsufficientInput();
        uint256 held = curveToken.balanceOf(msg.sender);
        if (perBuyerCap != 0 && held + out > perBuyerCap) revert ExcessivePurchase(held + out, perBuyerCap);
        if (out < minTokensOut) revert InsufficientOutput(minTokensOut, out);

        SafeToken.push(curveToken, msg.sender, out);
        emit Bought(msg.sender, netIn, out, fee, snipeTax);
    }

    /// @notice 커브 토큰을 팔아 quote를 받는다. brake와 무관하게 항상
    ///         열려 있다 — 탈출 경로는 막지 않는다.
    /// @param tokenIn      판매 토큰량 (pull — 도착량 기준 정산, F-02)
    /// @param minQuoteOut  슬리피지 하한 (수수료 차감 후 수령액 기준)
    function sell(uint256 tokenIn, uint256 minQuoteOut) external nonReentrant notClosed {
        uint256 preToken = curveToken.balanceOf(address(this));
        uint256 delivered = SafeToken.pull(curveToken, msg.sender, tokenIn);
        if (delivered == 0) revert InsufficientInput();

        uint256 out = _sellQuoteOut(delivered, preToken);
        uint256 fee = out * feeBps / _BPS_MAX;
        uint256 received = out - fee;
        if (received == 0) revert InsufficientInput();
        if (received < minQuoteOut) revert InsufficientOutput(minQuoteOut, received);

        SafeToken.push(quote, msg.sender, received);
        SafeToken.push(quote, treasury, fee);
        emit Sold(msg.sender, delivered, received, fee);
    }

    // ---------------------------------------------------------------- 졸업

    /// @notice 모금 목표 도달 시 누구나 호출. 커브 자산 전액 + 남은 토큰을
    ///         AMM 페어로 옮기고 LP를 dead 주소에 영구 잠근다.
    /// @dev 신규 페어 생성(또는 재사용)은 진입 행위로 분류해 brake에 걸린다.
    ///      brake 중에도 sell은 열려 있으므로 자금이 갇히지 않는다.
    function graduate() external nonReentrant whenEntryOpen {
        if (graduated) revert AlreadyGraduated();
        uint256 raised = quote.balanceOf(address(this));
        if (raised < graduationTarget) revert NotGraduable(raised, graduationTarget);

        AmmPair pair = _pairForGraduation();
        SafeToken.push(quote, address(pair), raised);
        uint256 tokens = curveToken.balanceOf(address(this));
        SafeToken.push(curveToken, address(pair), tokens);
        uint256 lp = pair.mint(_DEAD); // LP 영구 잠금 — 회수·판매 경로 없음

        graduated = true;
        graduatePair = address(pair);
        emit Graduated(address(pair), raised, tokens, lp);
    }

    /// @dev (token, quote) 페어를 가져온다. 없으면 만든다. 이미 있으면
    ///      LP 공급이 MINIMUM_LIQUIDITY 이하(사실상 dead 잠금분뿐)일 때만
    ///      재사용 — 선점 예치로 졸업 유동성을 훔치는 것을 막는다.
    function _pairForGraduation() private returns (AmmPair pair) {
        address existing = factory.getPair(address(curveToken), address(quote));
        if (existing == address(0)) {
            pair = AmmPair(factory.createPair(address(curveToken), address(quote)));
        } else {
            if (AmmPair(existing).totalSupply() > _MIN_LIQ) revert PairPoisoned(existing);
            pair = AmmPair(existing);
        }
    }

    // ---------------------------------------------------------------- 뷰 (F-04: 상수 시간)

    /// @notice 매수 견적 — fee/스나이프 세금 감안 후 실제 수령할 토큰량.
    function getBuyQuoteOut(uint256 quoteIn) external view returns (uint256) {
        (,, uint256 netIn) = _buyTaxes(quoteIn);
        return _buyTokenOut(netIn, quote.balanceOf(address(this)));
    }

    /// @notice 매도 견적 — 수수료 차감 후 실제 수령할 quote.
    function getSellQuoteOut(uint256 tokenIn) external view returns (uint256) {
        uint256 out = _sellQuoteOut(tokenIn, curveToken.balanceOf(address(this)));
        return out - (out * feeBps / _BPS_MAX);
    }

    /// @notice 현재 가상 리저브 (vq, vt). 오라클·UI용.
    function virtualReserves() external view returns (uint256 vq, uint256 vt) {
        vq = quote.balanceOf(address(this)) + quoteFloor;
        vt = curveToken.balanceOf(address(this)) + tokenFloor;
    }

    // ---------------------------------------------------------------- 내부

    /// @dev launch 직후 감쇠하는 스나이프 세금. window 안에서 선형,
    ///      이후 0. snipeTaxBps == 0이면 상태 없이 0.
    function _snipeTax(uint256 amount) private view returns (uint256) {
        if (snipeTaxBps == 0) return 0;
        uint256 elapsed = block.timestamp - launchTime;
        if (elapsed >= snipeWindow) return 0;
        return (amount * snipeTaxBps * (snipeWindow - elapsed)) / snipeWindow / _BPS_MAX;
    }

    function _buyTaxes(uint256 delivered) private view returns (uint256 fee, uint256 snipeTax, uint256 netIn) {
        fee = (delivered * feeBps) / _BPS_MAX;
        snipeTax = _snipeTax(delivered);
        netIn = delivered - fee - snipeTax;
    }

    /// @param netIn              세금 차감 후 유효 매수액
    /// @param quoteBalanceBefore 이번 매수 전 커브 quote 잔액 (뷰/본문 동일식)
    function _buyTokenOut(uint256 netIn, uint256 quoteBalanceBefore) private view returns (uint256) {
        uint256 vq = quoteBalanceBefore + quoteFloor;
        uint256 vt = curveToken.balanceOf(address(this)) + tokenFloor;
        return (vt * netIn) / (vq + netIn);
    }

    /// @param tokenIn            유효 매도량 (도착량)
    /// @param tokenBalanceBefore 이번 매도 전 커브 토큰 잔액
    function _sellQuoteOut(uint256 tokenIn, uint256 tokenBalanceBefore) private view returns (uint256) {
        uint256 vq = quote.balanceOf(address(this)) + quoteFloor;
        uint256 vt = tokenBalanceBefore + tokenFloor;
        return (vq * tokenIn) / (vt + tokenIn);
    }
}
