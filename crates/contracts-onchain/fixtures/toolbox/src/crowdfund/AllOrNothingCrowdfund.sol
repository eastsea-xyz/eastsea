// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";
import {SimpleBrake} from "src/system/SimpleBrake.sol";

/// @title 예제 12 — 올오어낫싱 크라우드펀드
/// @notice 마감까지 목표 금액을 채우면 전액이 수혜자에게 인도되고,
///         못 채우면 기여자 전원이 전액 환불받는다. 중간 지점은 없다.
/// @dev
///  설계 결정 — 상태는 기여 회계가 전부다:
///   - 목표 달성 판정은 스칼라 하나(raised >= goal) — 목록 순회
///     없이(F-04) 어느 시점이든 판정된다.
///   - 기여자당 슬롯 1개는 환불 회계에 필수다 — "누가 얼마 냈는가"를
///     잔액이나 이벤트에서 유도하면 안 된다(예제 7의 잔액 유도 함정과
///     같은 이유). 다만 이 슬롯은 **일시적**이다: 캠페인 실패 시
///     환불이 슬롯을 지운다(delete). 성공 시에만 기여자 수만큼
///     영구 상태로 남는다 — 그 상태의 가치(출처 증명)는 국고 회계로
///     필요하다.
///   - 조기 달성: 목표를 채우는 순간 contribute는 막히고(AlreadyFunded)
///     withdraw는 마감 전이라도 연다. 기여자는 "달성 = 성공"을
///     이미 알고 있으므로 마감까지 기다릴 이유가 없다.
///   - 초과 기여: 마지막 기여가 목표를 넘어도 전액 수용한다.
///     초과분은 후원으로 처리 — 초과분 비례 환불은 회계를 복잡하게
///     하고(부분 성공) all-or-nothing 정신이 아니다.
///
///  F 매핑:
///   F-01: refund/withdraw CEI(슬롯/플래그 갱신 먼저, native 전송
///         마지막) + nonReentrant. contribute는 외부 호출이 없다.
///   F-02/F-03: native만 — FoT·무코드 영역 밖. receive()는 거부.
///   F-04: 목표 달성 판정 스칼라 비교 — 상수 시간.
///   F-07: Contributed 이벤트 ≠ 기여 — 진실은 contributions 매핑.
///   F-08: 사용하지 않음.
contract AllOrNothingCrowdfund is SimpleBrake, ReentrancyGuard {
    /// @dev 수혜자 — 성공 시 수령인 (배포 시 고정)
    address public immutable beneficiary;

    /// @dev 목표 금액 (wei) — 달성 판정의 기준
    uint128 public immutable goal;

    /// @dev 마감 시각 — 이후 미달이면 환불 개방
    uint48 public immutable deadline;

    /// @custom:member raised    지금까지 모인 금액 (목표 판정 스칼라)
    /// @custom:member withdrawn 성공 인도 완료 — 1회
    struct Campaign {
        uint128 raised;
        bool withdrawn;
    }

    Campaign public campaign;

    /// @dev 기여자별 납입액 — 환불 회계. 실패 시 환불이 지운다(일시적 슬롯)
    mapping(address => uint128) public contributions;

    error ZeroAmount();
    error GoalTooLow();
    error DeadlineTooShort();
    error DeadlinePassed(uint256 at, uint256 until);
    error AlreadyFunded(uint256 raised, uint256 goal);
    error RefundNotAllowed(uint256 raised, uint256 goal, uint256 until);
    error NothingToRefund();
    error NothingToWithdraw();
    error NativeTransferFailed();

    event Contributed(address indexed user, uint128 amount, uint128 raised);
    event Refunded(address indexed user, uint128 amount);
    event Withdrawn(address indexed beneficiary, uint128 amount);

    /// @param beneficiary_ 성공 시 수령인
    /// @param goal_        목표 금액
    /// @param durationSec  마감까지의 기간 (배포 시각부터)
    constructor(address guardian, address beneficiary_, uint128 goal_, uint48 durationSec) SimpleBrake(guardian) {
        if (beneficiary_ == address(0)) revert ZeroAmount();
        if (goal_ == 0) revert GoalTooLow();
        if (durationSec == 0) revert DeadlineTooShort();

        beneficiary = beneficiary_;
        goal = goal_;
        deadline = uint48(block.timestamp) + durationSec;
    }

    /// @dev 직접 송금 거부 — 기여는 contribute의 회계를 거친다.
    receive() external payable {
        revert NativeTransferFailed();
    }

    // ---------------------------------------------------------------- 기여 (진입 — brake 차단)

    /// @notice 목표가 채울 때까지 기여한다. 마감 후·달성 후 거부.
    function contribute() external payable nonReentrant whenEntryOpen {
        if (msg.value == 0) revert ZeroAmount();
        if (block.timestamp > deadline) revert DeadlinePassed(block.timestamp, deadline);

        uint128 raised = campaign.raised;
        if (raised >= goal) revert AlreadyFunded(raised, goal);

        // 목표를 넘는 마지막 기여도 전압 수용 — 초과분은 후원
        uint128 newRaised = raised + uint128(msg.value);
        campaign.raised = newRaised;
        contributions[msg.sender] += uint128(msg.value);
        emit Contributed(msg.sender, uint128(msg.value), newRaised);
    }

    // ---------------------------------------------------------------- 환불 (탈출 — brake 무관)

    /// @notice 캠페인 실패(마감 + 목표 미달) 시 전액 환불받는다.
    ///         환불은 기여 슬롯을 지운다 — 캠페인의 상태는 정리된다.
    function refund() external nonReentrant {
        uint128 c = contributions[msg.sender];
        if (c == 0) revert NothingToRefund();

        uint128 raised = campaign.raised;
        // 성공했거나 아직 진행중이면 환불 없다 — one-shot 판정
        if (raised >= goal || block.timestamp <= deadline) {
            revert RefundNotAllowed(raised, goal, deadline);
        }

        delete contributions[msg.sender]; // CEI: 슬롯 소거 먼저
        (bool ok,) = msg.sender.call{value: c}("");
        if (!ok) revert NativeTransferFailed();
        emit Refunded(msg.sender, c);
    }

    // ---------------------------------------------------------------- 인도 (탈출 — brake 무관)

    /// @notice 목표 달성 시 모금액 전액을 beneficiary에게 인도한다.
    ///         누구나 호출 가능 (가스 대낭) — 수령인은 항상 beneficiary.
    function withdraw() external nonReentrant {
        Campaign storage c = campaign;
        if (c.withdrawn || c.raised < goal) revert NothingToWithdraw();

        uint128 amount = c.raised;
        c.withdrawn = true; // CEI: 플래그 먼저
        (bool ok,) = beneficiary.call{value: amount}("");
        if (!ok) revert NativeTransferFailed();
        emit Withdrawn(beneficiary, amount);
    }

    // ---------------------------------------------------------------- 뷰 (F-04)

    /// @dev 0 진행중, 1 성공(달성), 2 실패(마감+미달), 3 성공·인도 완료
    function state() external view returns (uint8) {
        Campaign storage c = campaign;
        if (c.raised >= goal) return c.withdrawn ? 3 : 1;
        if (block.timestamp > deadline) return 2;
        return 0;
    }
}
