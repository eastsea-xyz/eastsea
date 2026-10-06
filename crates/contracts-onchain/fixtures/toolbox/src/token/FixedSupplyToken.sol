// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC20} from "openzeppelin/token/ERC20/ERC20.sol";
import {ERC20Permit} from "openzeppelin/token/ERC20/extensions/ERC20Permit.sol";

/// @title 예제 1 — 고정 공급 ERC-20 (EIP-2612 permit 포함)
/// @notice 생성 시 전량이 수령인에게 한 번 발행되고, 이후 추가 발행 경로가
///         아예 없다. mint 함수가 존재하지 않는 것이 이 예제의 계약이다.
/// @dev
///  상태 비용 (docs/paid-state-design.md 참조):
///   - 배포 시 코드 바이트 약 1 unit/byte (permit 포함 시 증가).
///   - transfer는 상대방 잔액 슬롯이 0이던 경우 새 슬롯 1개 = 100 units.
///     기존 수신자에게 재전송은 새 슬롯 없음.
///   - approve는 허용량 슬롯 1개. permit은 논스 슬롯도 1개.
///   - Transfer 이벤트 3 토픽(from,to,시그니처) + 32B data(value) = 64+96+32
///     = 192 계량 바이트 = 6 units. 실측과 일치 (GAS.md 참조).
contract FixedSupplyToken is ERC20, ERC20Permit {
    error ZeroRecipient();
    error ZeroSupply();

    /// @param name_     토큰 이름 (예: "Island Coin")
    /// @param symbol_   심볼 (예: "ISLE")
    /// @param supply    총 공급 (18 decimals 기준 원하는 양)
    /// @param recipient 최초 전량 수령인. 이후 분배는 오직 transfer로.
    constructor(string memory name_, string memory symbol_, uint256 supply, address recipient)
        ERC20(name_, symbol_)
        ERC20Permit(name_)
    {
        if (recipient == address(0)) revert ZeroRecipient();
        if (supply == 0) revert ZeroSupply();
        _mint(recipient, supply);
    }
}
