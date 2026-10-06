// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "openzeppelin/token/ERC20/IERC20.sol";
import {SimpleBrake} from "src/system/SimpleBrake.sol";
import {AmmPair} from "./AmmPair.sol";

/// @notice 예제 4 — 상수곱(constant-product) AMM 팩토리.
/// @dev
///  페어 생성은 신규 진입이므로 brake(>= 1)에서 차단된다. 이미 만들어진
///  페어의 탈출(burn)은 팩토리 brake와 무관하게 항상 열려 있다 — 페어
///  컨트랙트가 이 규칙을 적용한다.
///  F-03: 무코드 주소로 페어를 만들 수 없다. 배포된 코드가 없는 토큰은
///  이체 자체가 불가능하고, 페어만 남아 상태 비용을 태운다.
contract AmmFactory is SimpleBrake {
    /// @dev tokenA/tokenB (정렬된) -> pair. 없으면 address(0).
    mapping(address => mapping(address => address)) public getPair;
    address[] public allPairs;

    error IdenticalTokens(address token);
    error ZeroTokenAddress();
    error TokenHasNoCode(address token);
    error PairExists(address pair);

    event PairCreated(address indexed token0, address indexed token1, address pair, uint256 pairCount);

    constructor(address guardian) SimpleBrake(guardian) {}

    function allPairsLength() external view returns (uint256) {
        return allPairs.length;
    }

    /// @notice 새 페어를 만든다. tokenA/tokenB 순서는 임의로 넣어도 된다.
    function createPair(address tokenA, address tokenB) external whenEntryOpen returns (address pair) {
        if (tokenA == tokenB) revert IdenticalTokens(tokenA);
        (address token0, address token1) = tokenA < tokenB ? (tokenA, tokenB) : (tokenB, tokenA);
        if (token0 == address(0)) revert ZeroTokenAddress();
        // F-03: 코드 없는 토큰은 거부 — 페어 생성 후 첫 예치에서야 죽는
        // 좀비 페어를 만들지 않는다.
        if (token0.code.length == 0) revert TokenHasNoCode(token0);
        if (token1.code.length == 0) revert TokenHasNoCode(token1);
        if (getPair[token0][token1] != address(0)) revert PairExists(getPair[token0][token1]);

        pair = address(new AmmPair(IERC20(token0), IERC20(token1), this));
        getPair[token0][token1] = pair;
        getPair[token1][token0] = pair;
        allPairs.push(pair);
        emit PairCreated(token0, token1, pair, allPairs.length);
    }
}
