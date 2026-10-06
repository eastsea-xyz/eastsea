// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "openzeppelin/token/ERC20/IERC20.sol";
import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";
import {SafeToken} from "src/common/SafeToken.sol";
import {AmmFactory} from "./AmmFactory.sol";
import {AmmPair} from "./AmmPair.sol";

/// @notice 예제 4 — AMM 라우터. 유동성 공급·회수·경로 스왑의 진입점.
/// @dev
///  토큰 이동은 SafeToken(F-02/F-03 대응)으로만 한다. 페어가 없으면
///  addLiquidity 안에서 만든다. native(AETH) 스왑은 다루지 않는다 —
///  필요하면 래핑 토큰을 경로에 넣어라 (래핑 자체가 별도 예제 주제).
///  F-04: 경로 길이는 호출자가 준 path[] 배열에 한정된다 — 순회 뷰가
///  아니다. getAmountsOut은 path.length만큼만 계산한다.
contract AmmRouter is ReentrancyGuard {
    using SafeToken for IERC20;

    AmmFactory public immutable factory;

    /// @dev AmmPair.FEE_NUMERATOR와 짝 — 상수는 타입 멤버로 접근할 수 없어 복제한다.
    uint256 private constant FEE_NUMERATOR = 997;
    uint256 private constant FEE_DENOMINATOR = 1000;

    error Expired();
    error InsufficientAmount();
    error InsufficientAmountA();
    error InsufficientAmountB();
    error ExcessiveAmountA();
    error ExcessiveAmountB();
    error InsufficientOutput(uint256 expected, uint256 actual);
    error InvalidPath();

    event LiquidityAdded(address indexed sender, address indexed pair, uint256 amountA, uint256 amountB);
    event LiquidityRemoved(address indexed sender, address indexed pair, uint256 amountA, uint256 amountB);

    constructor(AmmFactory _factory) {
        factory = _factory;
    }

    // ---- 유동성 ----

    /// @notice 페어가 없으면 만들고, 원하는 비율로 유동성을 공급한다.
    /// @dev 비율 계산은 Uniswap V2와 동일한 quote 기준. 실제 예치는
    ///      SafeToken.pull의 도착량(delta) 기준 — F-02 안전.
    function addLiquidity(
        address tokenA,
        address tokenB,
        uint256 amountADesired,
        uint256 amountBDesired,
        uint256 amountAMin,
        uint256 amountBMin,
        address to,
        uint256 deadline
    ) external nonReentrant returns (uint256 amountA, uint256 amountB, uint256 liquidity) {
        if (block.timestamp > deadline) revert Expired();
        (AmmPair pair, uint256 reserveA, uint256 reserveB) = _pairForOrCreate(tokenA, tokenB);
        (amountA, amountB) = _optimalAmounts(amountADesired, amountBDesired, amountAMin, amountBMin, reserveA, reserveB);

        _fundPair(tokenA, tokenB, address(pair), amountA, amountB);
        liquidity = pair.mint(to);
        emit LiquidityAdded(msg.sender, address(pair), amountA, amountB);
    }

    /// @dev 사용자로부터 두 토큰을 끌어와 페어로 전달한다 (잔여분 포함).
    function _fundPair(address tokenA, address tokenB, address pair, uint256 amountA, uint256 amountB) private {
        IERC20(tokenA).pull(msg.sender, amountA);
        IERC20(tokenB).pull(msg.sender, amountB);
        IERC20(tokenA).push(pair, IERC20(tokenA).balanceOf(address(this)));
        IERC20(tokenB).push(pair, IERC20(tokenB).balanceOf(address(this)));
    }

    /// @dev 현재 리저브 비율에 맞춰 실제 예치액을 결정한다 (Uniswap V2 방식).
    function _optimalAmounts(
        uint256 amountADesired,
        uint256 amountBDesired,
        uint256 amountAMin,
        uint256 amountBMin,
        uint256 reserveA,
        uint256 reserveB
    ) private pure returns (uint256 amountA, uint256 amountB) {
        if (reserveA == 0 && reserveB == 0) {
            return (amountADesired, amountBDesired);
        }
        uint256 amountBOptimal = (amountADesired * reserveB) / reserveA;
        if (amountBOptimal <= amountBDesired) {
            if (amountBOptimal < amountBMin) revert ExcessiveAmountA();
            return (amountADesired, amountBOptimal);
        }
        uint256 amountAOptimal = (amountBDesired * reserveA) / reserveB;
        if (amountAOptimal < amountAMin) revert ExcessiveAmountB();
        return (amountAOptimal, amountBDesired);
    }

    /// @notice LP를 반납하고 토큰을 회수한다. 탈출 — brake와 무관.
    function removeLiquidity(
        address tokenA,
        address tokenB,
        uint256 liquidity,
        uint256 amountAMin,
        uint256 amountBMin,
        address to,
        uint256 deadline
    ) external nonReentrant returns (uint256 amountA, uint256 amountB) {
        if (block.timestamp > deadline) revert Expired();
        AmmPair pair = _mustPair(tokenA, tokenB);

        IERC20(address(pair)).pull(msg.sender, liquidity);
        IERC20(address(pair)).push(address(pair), liquidity);
        (amountA, amountB) = pair.burn(to);

        if (amountA < amountAMin) revert InsufficientOutput(amountAMin, amountA);
        if (amountB < amountBMin) revert InsufficientOutput(amountBMin, amountB);
        emit LiquidityRemoved(msg.sender, address(pair), amountA, amountB);
    }

    // ---- 스왑 ----

    /// @notice path[0]을 정확히 amountIn 지불해 path[last]를 최소 amountOutMin 수령.
    function swapExactTokensForTokens(
        uint256 amountIn,
        uint256 amountOutMin,
        address[] calldata path,
        address to,
        uint256 deadline
    ) external nonReentrant returns (uint256[] memory amounts) {
        if (block.timestamp > deadline) revert Expired();
        amounts = getAmountsOut(amountIn, path);
        if (amounts[amounts.length - 1] < amountOutMin) {
            revert InsufficientOutput(amountOutMin, amounts[amounts.length - 1]);
        }

        IERC20(path[0]).pull(msg.sender, amounts[0]);
        IERC20(path[0]).push(address(_mustPair(path[0], path[1])), amounts[0]);
        _swap(amounts, path, to);
    }

    /// @notice FoT 토큰용 스왑 — 홉별 출력량을 견적 대신 페어의 실제 도착
    ///         잔액에서 관찰해 계산한다 (Uniswap V2의 supporting-fee 변형).
    /// @dev 마지막 홉에서도 최종 수령량을 다시 관찰할 수 없다(수신자 주소의
    ///      사전 잔액을 모른다). amountOutMin은 페어가 준 양 기준으로 검증된다 —
    ///      수신자에게 가는 마지막 이체에서 또 과징하는 토큰은 이 라우터의
    ///      관심 밖이다(문서 참조).
    function swapSupportingFeeOnTransfer(
        uint256 amountIn,
        uint256 amountOutMin,
        address[] calldata path,
        address to,
        uint256 deadline
    ) external nonReentrant {
        if (block.timestamp > deadline) revert Expired();
        if (path.length < 2 || path.length > 4) revert InvalidPath();

        IERC20(path[0]).pull(msg.sender, amountIn);
        IERC20(path[0]).push(address(_mustPair(path[0], path[1])), IERC20(path[0]).balanceOf(address(this)));

        uint256 amountOut;
        for (uint256 i; i < path.length - 1; ++i) {
            bool lastHop = i == path.length - 2;
            address recipient = lastHop ? to : address(_mustPair(path[i + 1], path[i + 2]));
            amountOut = _observedHop(path[i], path[i + 1], recipient);
        }
        if (amountOut < amountOutMin) revert InsufficientOutput(amountOutMin, amountOut);
    }

    /// @dev 페어의 실제 도착량(reserve 대비 잔액 증가)으로 홉을 실행한다.
    function _observedHop(address input, address output, address recipient) private returns (uint256 amountOut) {
        AmmPair pair = _mustPair(input, output);
        (uint112 r0, uint112 r1,) = pair.getReserves();
        bool inputIsToken0 = input < output;
        (uint256 reserveIn, uint256 reserveOut) =
            inputIsToken0 ? (uint256(r0), uint256(r1)) : (uint256(r1), uint256(r0));
        uint256 amountInput = IERC20(input).balanceOf(address(pair)) - reserveIn;
        amountOut = getAmountOut(amountInput, reserveIn, reserveOut);
        (uint256 amount0Out, uint256 amount1Out) = inputIsToken0 ? (uint256(0), amountOut) : (amountOut, uint256(0));
        pair.swap(amount0Out, amount1Out, recipient);
    }

    /// @dev amounts는 이미 계산된 홉별 출력량. 중간 홉의 수령인은 다음
    ///      페어 주소다 (라우터를 거치지 않는다 — 가스·재진입 표면 최소화).
    ///      마지막 홉만 to에게 간다.
    function _swap(uint256[] memory amounts, address[] calldata path, address to) private {
        for (uint256 i; i < path.length - 1; ++i) {
            (address input, address output) = (path[i], path[i + 1]);
            AmmPair pair = _mustPair(input, output);
            (uint256 amount0Out, uint256 amount1Out) =
                input < output ? (uint256(0), amounts[i + 1]) : (amounts[i + 1], uint256(0));
            address recipient = i < path.length - 2 ? address(_mustPair(output, path[i + 2])) : to;
            pair.swap(amount0Out, amount1Out, recipient);
        }
    }

    // ---- 조회 ----

    /// @notice 견적. path는 [tokenIn, ..., tokenOut]. F-04: 배열 길이 한정.
    function getAmountsOut(uint256 amountIn, address[] calldata path) public view returns (uint256[] memory amounts) {
        if (path.length < 2 || path.length > 4) revert InvalidPath();
        amounts = new uint256[](path.length);
        amounts[0] = amountIn;
        for (uint256 i; i < path.length - 1; ++i) {
            (, uint256 reserveIn, uint256 reserveOut) = _reservesOf(path[i], path[i + 1]);
            amounts[i + 1] = getAmountOut(amounts[i], reserveIn, reserveOut);
        }
    }

    /// @notice 상수곱 출력량 (수수료 0.30% 반영).
    function getAmountOut(uint256 amountIn, uint256 reserveIn, uint256 reserveOut)
        public
        pure
        returns (uint256 amountOut)
    {
        if (amountIn == 0) revert InsufficientAmount();
        if (reserveIn == 0 || reserveOut == 0) revert InsufficientLiquidityPair();
        uint256 amountInWithFee = amountIn * FEE_NUMERATOR;
        amountOut = (amountInWithFee * reserveOut) / (reserveIn * FEE_DENOMINATOR + amountInWithFee);
    }

    // ---- 내부 ----

    error InsufficientLiquidityPair();
    error PairNotFound(address tokenA, address tokenB);

    function _pairForOrCreate(address tokenA, address tokenB)
        private
        returns (AmmPair pair, uint256 reserveA, uint256 reserveB)
    {
        address existing = factory.getPair(tokenA, tokenB);
        if (existing == address(0)) {
            pair = AmmPair(factory.createPair(tokenA, tokenB));
        } else {
            pair = AmmPair(existing);
        }
        (uint112 r0, uint112 r1,) = pair.getReserves();
        (reserveA, reserveB) = tokenA < tokenB ? (uint256(r0), uint256(r1)) : (uint256(r1), uint256(r0));
    }

    function _mustPair(address tokenA, address tokenB) private view returns (AmmPair) {
        address pair = factory.getPair(tokenA, tokenB);
        if (pair == address(0)) revert PairNotFound(tokenA, tokenB);
        return AmmPair(pair);
    }

    function _reservesOf(address tokenA, address tokenB)
        private
        view
        returns (AmmPair pair, uint256 reserveA, uint256 reserveB)
    {
        pair = _mustPair(tokenA, tokenB);
        (uint112 r0, uint112 r1,) = pair.getReserves();
        (reserveA, reserveB) = tokenA < tokenB ? (uint256(r0), uint256(r1)) : (uint256(r1), uint256(r0));
    }
}
