// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "openzeppelin/token/ERC20/IERC20.sol";

/// @notice 감사 결과 F-01/F-02/F-03에 대한 toolbox 공용 ERC-20 안전 프리미티브.
/// @dev
///  F-03 (무코드·불성실 토큰): 모든 이동 전에 token.code.length > 0을 검사하고,
///       반환 데이터가 있으면 bool을 엄격히 확인한다. 빈 반환은 메인 레포 관례를
///       따라 성공으로 본다(일부 구형 토큰). 단 무코드 주소는 즉시 revert.
///  F-02 (수수료 과징 토큰): pull()은 실제 도착량(delivered)을 잔액 차이로 측정해
///       돌려준다. 약속 금액과 정확히 일치해야 하는 흐름은 pullExact()를 쓴다.
///  F-01 (재진입 예치): 이 라이브러리 자체는 가드가 아니다. pull을 호출하는 모든
///       진입점에는 OZ ReentrancyGuard의 nonReentrant를 붙여야 한다. balanceOf
///       측정은 외부 호출 이후에 하므로, 가드가 없으면 중첩 예치가 delta에
///       섞여 들어온다 — 그것이 F-01의 원본 결함이다.
library SafeToken {
    error TokenHasNoCode(address token);
    error TokenTransferFailed(address token);
    error ShortDelivery(uint256 expected, uint256 delivered);

    /// @dev from -> this 로 amount를 끌어오고 실제 도착량을 측정해 돌려준다.
    function pull(IERC20 token, address from, uint256 amount) internal returns (uint256 delivered) {
        if (amount == 0) return 0;
        if (address(token).code.length == 0) revert TokenHasNoCode(address(token));
        uint256 before = token.balanceOf(address(this));
        (bool ok, bytes memory ret) =
            address(token).call(abi.encodeCall(IERC20.transferFrom, (from, address(this), amount)));
        if (!ok) revert TokenTransferFailed(address(token));
        if (ret.length != 0 && !abi.decode(ret, (bool))) revert TokenTransferFailed(address(token));
        delivered = token.balanceOf(address(this)) - before;
        if (delivered == 0) revert ShortDelivery(amount, 0); // 이동 없이 성공을 가장하는 토큰
    }

    /// @dev pull + 도착량 == amount 강제. 캠페인 예산·LP 예치처럼 정확한 금액이
    ///      계약 불변식에 들어가는 곳에만 쓴다(콜드 balanceOf 1회 추가).
    function pullExact(IERC20 token, address from, uint256 amount) internal {
        uint256 delivered = pull(token, from, amount);
        if (delivered != amount) revert ShortDelivery(amount, delivered);
    }

    /// @dev this -> to 로 amount를 보낸다. code 검사 + strict bool.
    function push(IERC20 token, address to, uint256 amount) internal {
        if (amount == 0) return;
        if (address(token).code.length == 0) revert TokenHasNoCode(address(token));
        (bool ok, bytes memory ret) = address(token).call(abi.encodeCall(IERC20.transfer, (to, amount)));
        if (!ok) revert TokenTransferFailed(address(token));
        if (ret.length != 0 && !abi.decode(ret, (bool))) revert TokenTransferFailed(address(token));
    }

    /// @dev push + 수취인 잔액 차이 검증. "지급했다"는 사실 자체가 계약상 약속인
    ///      곳(마켓 정산·래플 상금)에서 쓴다. 사악한 토큰이 true만 반환하고
    ///      이동하지 않는 경우를 잡는다(F-03).
    function pushExact(IERC20 token, address to, uint256 amount) internal {
        if (amount == 0) return;
        if (to.code.length != 0) {
            uint256 before = token.balanceOf(to);
            push(token, to, amount);
            if (token.balanceOf(to) - before != amount) revert ShortDelivery(amount, 0);
        } else {
            push(token, to, amount); // EOA 수취인은 잔액 조회가 무의미한 토큰도 있다
        }
    }
}
