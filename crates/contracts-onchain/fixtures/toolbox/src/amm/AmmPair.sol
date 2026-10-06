// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC20} from "openzeppelin/token/ERC20/ERC20.sol";
import {IERC20} from "openzeppelin/token/ERC20/IERC20.sol";
import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";
import {AmmFactory} from "./AmmFactory.sol";

/// @notice 예제 4 — 상수곱 페어. LP 토큰은 일반 ERC-20이다.
/// @dev
///  유입량은 전부 잔액 차이(balance - reserve)로 측정한다 — F-02:
///  수수료 과징(fee-on-transfer) 토큰이 약속 금액보다 적게 도착해도
///  실제 도착량만큼만 LP/스왑이 정산되므로 좌초하지 않는다.
///  팩토리 brake: state 1 이상이면 mint(예치)과 swap(진입)이 차단되고
///  burn(회수)·skim(잔여 회수)은 항상 열려 있다. 탈출은 막지 않는다.
///  TWAP: price{0,1}CumulativeLast는 reserve 기반 순간가격의 시간 적산.
///  소비자는 두 시점의 차이를 경과 시간으로 나눠 평균가를 얻는다.
contract AmmPair is ERC20, ReentrancyGuard {
    uint256 public constant MINIMUM_LIQUIDITY = 1000;
    /// @dev 스왑 수수료 0.30% — getAmountOut의 997/1000와 짝을 이룬다.
    uint256 public constant FEE_NUMERATOR = 997;
    uint256 public constant FEE_DENOMINATOR = 1000;
    /// @dev 최소 유동성 잠금처 — 프라이빗 키가 없는 관례 주소.
    address private constant _DEAD = 0x000000000000000000000000000000000000dEaD;

    AmmFactory public immutable factory;
    IERC20 public immutable token0;
    IERC20 public immutable token1;

    /// @dev slot 1: reserve 2개 + 타임스탬프 (Uniswap V2 레이아웃)
    uint112 private reserve0;
    uint112 private reserve1;
    uint32 private blockTimestampLast;
    /// @dev slot 2-3: TWAP 적산치
    uint256 public price0CumulativeLast;
    uint256 public price1CumulativeLast;

    error NotBraked(uint8 state);
    error InsufficientOutput();
    error ExcessiveOutput();
    error InsufficientLiquidity();
    error InsufficientLiquidityMinted();
    error InsufficientLiquidityBurned();
    error InvalidSwapAmounts();
    error TransferFailed();
    error ReserveOverflow();
    error FactoryBrakedNewEntry(uint8 state);
    error FactoryBrakedFull(uint8 state);

    event Mint(address indexed sender, uint256 amount0, uint256 amount1);
    event Burn(address indexed sender, address indexed to, uint256 amount0, uint256 amount1);
    event Swap(
        address indexed sender,
        address indexed to,
        uint256 amount0In,
        uint256 amount1In,
        uint256 amount0Out,
        uint256 amount1Out
    );
    event Sync(uint112 reserve0, uint112 reserve1);

    modifier factoryEntryOpen() {
        (uint8 state,,) = factory.brakeState();
        if (state >= 1) revert FactoryBrakedNewEntry(state);
        _;
    }

    constructor(IERC20 _token0, IERC20 _token1, AmmFactory _factory) ERC20("EastSea AMM LP", "EALP") {
        factory = _factory;
        token0 = _token0;
        token1 = _token1;
    }

    function getReserves() public view returns (uint112 _reserve0, uint112 _reserve1, uint32 _blockTimestampLast) {
        (_reserve0, _reserve1) = (reserve0, reserve1);
        _blockTimestampLast = blockTimestampLast;
    }

    /// @notice 예치(라우터가 토큰을 미리 보낸 뒤 호출). LP는 to에게.
    function mint(address to) external nonReentrant factoryEntryOpen returns (uint256 liquidity) {
        // F-02: 유입량 = 잔액 - 리저브 (약속이 아니라 도착량 기준)
        (uint112 _reserve0, uint112 _reserve1,) = getReserves();
        uint256 balance0 = token0.balanceOf(address(this));
        uint256 balance1 = token1.balanceOf(address(this));
        uint256 amount0 = balance0 - _reserve0;
        uint256 amount1 = balance1 - _reserve1;

        uint256 _totalSupply = totalSupply();
        if (_totalSupply == 0) {
            liquidity = _sqrt(amount0 * amount1);
            // 최소 유동성보다 작으면 첫 예치를 거절 (0 유입 underflow 방지)
            if (liquidity <= MINIMUM_LIQUIDITY) revert InsufficientLiquidityMinted();
            liquidity -= MINIMUM_LIQUIDITY;
            // 최소 유동성은 영구 잠김 — 키 없는 dead 주소로 발행한다.
            // (OZ v5는 address(0) 수신을 금지한다)
            _mint(_DEAD, MINIMUM_LIQUIDITY);
        } else {
            liquidity = _min((amount0 * _totalSupply) / _reserve0, (amount1 * _totalSupply) / _reserve1);
        }
        if (liquidity == 0) revert InsufficientLiquidityMinted();
        _mint(to, liquidity);

        _update(balance0, balance1);
        emit Mint(msg.sender, amount0, amount1);
    }

    /// @notice 회수(라우터가 LP를 미리 보낸 뒤 호출). 토큰은 to에게 직접.
    function burn(address to) external nonReentrant returns (uint256 amount0, uint256 amount1) {
        // 탈출 경로 — 팩토리 brake와 무관하게 항상 열려 있다
        (uint112 _reserve0, uint112 _reserve1,) = getReserves();
        uint256 balance0 = token0.balanceOf(address(this));
        uint256 balance1 = token1.balanceOf(address(this));
        uint256 liquidity = balanceOf(address(this));

        amount0 = (liquidity * balance0) / totalSupply();
        amount1 = (liquidity * balance1) / totalSupply();
        if (amount0 == 0 || amount1 == 0) revert InsufficientLiquidityBurned();
        _burn(address(this), liquidity);

        // F-03: 이체 대상 토큰은 페어 생성 시 코드가 확인됐다. 그래도
        // 반환값 strict 확인 — ERC-20 위반 토큰은 여기서 죽는다.
        (bool ok0, bytes memory ret0) = address(token0).call(abi.encodeCall(IERC20.transfer, (to, amount0)));
        if (!ok0 || (ret0.length != 0 && !abi.decode(ret0, (bool)))) revert TransferFailed();
        (bool ok1, bytes memory ret1) = address(token1).call(abi.encodeCall(IERC20.transfer, (to, amount1)));
        if (!ok1 || (ret1.length != 0 && !abi.decode(ret1, (bool)))) revert TransferFailed();

        balance0 = token0.balanceOf(address(this));
        balance1 = token1.balanceOf(address(this));
        _update(balance0, balance1);
        emit Burn(msg.sender, to, amount0, amount1);
    }

    /// @notice 스왑. 정확히 한쪽 out만 0이 아니어야 하고, 수령인은 to.
    /// @dev 라우터가 out 계산을 책임진다. 페어는 k 보존만 강제한다.
    function swap(uint256 amount0Out, uint256 amount1Out, address to) external nonReentrant factoryEntryOpen {
        if (amount0Out == 0 && amount1Out == 0) revert InvalidSwapAmounts();
        (uint112 _reserve0, uint112 _reserve1,) = getReserves();
        if (amount0Out >= _reserve0 || amount1Out >= _reserve1) revert InsufficientLiquidity();

        // out 먼저 전달 (이후 k 검증이 남아 있다 — 재진입 이득 없음)
        if (amount0Out > 0) _safeTransfer(token0, to, amount0Out);
        if (amount1Out > 0) _safeTransfer(token1, to, amount1Out);

        // F-02: 유입량은 잔액 차이. k 보존 검증까지 _settleSwap에 위임.
        (uint256 amount0In, uint256 amount1In) = _settleSwap(_reserve0, _reserve1, amount0Out, amount1Out);

        _update(token0.balanceOf(address(this)), token1.balanceOf(address(this)));
        emit Swap(msg.sender, to, amount0In, amount1In, amount0Out, amount1Out);
    }

    /// @dev 스왑 후 잔액을 읽어 유입량을 계산하고, 수수료 보정 k 비감소를
    ///      강제한다. 유입량을 돌려준다 (이벤트용).
    function _settleSwap(uint112 reserve0_, uint112 reserve1_, uint256 amount0Out, uint256 amount1Out)
        private
        view
        returns (uint256 amount0In, uint256 amount1In)
    {
        uint256 balance0 = token0.balanceOf(address(this));
        uint256 balance1 = token1.balanceOf(address(this));
        amount0In = balance0 > reserve0_ - amount0Out ? balance0 - (reserve0_ - amount0Out) : 0;
        amount1In = balance1 > reserve1_ - amount1Out ? balance1 - (reserve1_ - amount1Out) : 0;
        if (amount0In == 0 && amount1In == 0) revert InsufficientOutput();

        // k 비감소 검증 — Uniswap V2 정식: 양쪽 밸런스에 1000 스케일,
        // 유입쪽에서 수수료(3/1000) 차감. 기준은 스왑 전 리저브의 곱.
        uint256 balance0Adjusted = balance0 * FEE_DENOMINATOR - (amount0In * (FEE_DENOMINATOR - FEE_NUMERATOR));
        uint256 balance1Adjusted = balance1 * FEE_DENOMINATOR - (amount1In * (FEE_DENOMINATOR - FEE_NUMERATOR));
        if (
            balance0Adjusted * balance1Adjusted
                < uint256(reserve0_) * uint256(reserve1_) * FEE_DENOMINATOR * FEE_DENOMINATOR
        ) {
            revert ExcessiveOutput();
        }
    }

    /// @notice 잔액-리저브 초과분(우연 입금 등)을 to로 꺼낸다. sync의 안전한 짝.
    function skim(address to) external nonReentrant {
        (uint112 _reserve0, uint112 _reserve1,) = getReserves();
        _safeTransfer(token0, to, token0.balanceOf(address(this)) - _reserve0);
        _safeTransfer(token1, to, token1.balanceOf(address(this)) - _reserve1);
    }

    /// @notice 리저브를 현재 잔액으로 강제 재동기화. 토큰이 잔액을 스스로
    ///         늘린 뒤 복구하는 비상구. k는 줄어들 수 있다 — 문서 참조.
    function sync() external nonReentrant {
        _update(token0.balanceOf(address(this)), token1.balanceOf(address(this)));
    }

    // ---- 내부 ----

    function _safeTransfer(IERC20 token, address to, uint256 amount) private {
        (bool ok, bytes memory ret) = address(token).call(abi.encodeCall(IERC20.transfer, (to, amount)));
        if (!ok || (ret.length != 0 && !abi.decode(ret, (bool)))) revert TransferFailed();
    }

    function _update(uint256 balance0, uint256 balance1) private {
        if (balance0 > type(uint112).max || balance1 > type(uint112).max) revert ReserveOverflow();
        (uint112 _reserve0, uint112 _reserve1, uint32 blockTimestampLast_) = getReserves();

        // TWAP: 직전 갱신 이후 경과 시간만큼 순간가격을 적산한다.
        uint32 timeElapsed = uint32(block.timestamp) - blockTimestampLast_;
        if (timeElapsed > 0 && _reserve0 != 0 && _reserve1 != 0) {
            price0CumulativeLast += (uint256(_reserve1) * 1e18 / _reserve0) * timeElapsed;
            price1CumulativeLast += (uint256(_reserve0) * 1e18 / _reserve1) * timeElapsed;
        }

        reserve0 = uint112(balance0);
        reserve1 = uint112(balance1);
        blockTimestampLast = uint32(block.timestamp);
        emit Sync(reserve0, reserve1);
    }

    function _sqrt(uint256 y) private pure returns (uint256 z) {
        if (y > 3) {
            z = y;
            uint256 x = y / 2 + 1;
            while (x < z) {
                z = x;
                x = (y / x + x) / 2;
            }
        } else if (y != 0) {
            z = 1;
        }
    }

    function _min(uint256 a, uint256 b) private pure returns (uint256) {
        return a < b ? a : b;
    }
}
