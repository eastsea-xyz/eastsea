// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";
import {SimpleBrake} from "src/system/SimpleBrake.sol";

/// @title 예제 10 — 구독 관리자
/// @notice 초당 요율로 시간을 사는 선불 기간권. 보낸 금액만큼 만료
///         시각이 늘어나고, 취소하면 남은 초를 비례 환불한다.
///         소비된 시간은 운영 수익이 되어 누구나 정산 트리거로
///         수취인에게 인도할 수 있다.
/// @dev
///  설계 결정 — 운영자 키가 없다:
///   - ratePerSecond는 생성자 고정(immutable). 요금 변경은 새
///     컨트랙트 배포로 한다 — 요금 조작 권한 자체가 공격면이다.
///   - guardian은 brake(신규 구독 차단)만 가능. 자금·기간 변경 불가.
///   - payee는 수익 수취 주소(immutable). claimRevenue는 permissionless
///     — 지갑이 행동하지 않아도 누구나 정산을 대신 트리거한다.
///
///  회계 — 대칭 산수와 준비금 스칼라:
///   subscribe: seconds = msg.value / ratePerSecond (버림)
///   cancel:    refund  = remainingSec * ratePerSecond
///   지불과 환불이 같은 곱셈식이라 dust가 양쪽에서 1초 미만으로
///   대칭을 이룬다. 잔여 <1초치(value % rate)는 기간에 못 들어가
///   거부 없이 흡수된다 — 정확 배수를 강제하는 것보다 UX 오류가 적다.
///
///   소비 수익의 회수가 설계의 관건이다. 모든 활성 구독의 "납입
///   원금" 합(refundReserve)을 글로벌 스칼라 하나로 유지하면:
///     인도 가능 수익 = balance - refundReserve
///   을 사용자 목록 반복 없이(O(1), F-04) 계산할 수 있다. 준비금은
///   cancel/settle에서만 줄어든다 — 시간이 흘러 실제 부채가 더 줄어도
///   값은 그대로라 항상 보수적(상한)이다. 환불은 언제나 전액 뒷받침된다.
///
///  상태 — 사용자당 1슬롯:
///   Subscriber { uint64 expiry; uint88 contributed; } 팩 — 만료 시각과
///   이번 이어달기 납입액이 32바이트에 함께 산다. 기간 연장은 재기록,
///   새 슬롯은 첫 구독 1회뿐이다.
///
///  F 매핑:
///   F-01: subscribe/cancel/claim nonReentrant + CEI(스토리지 먼저,
///         native 전송 마지막). settle/settleExpired는 외부 호출 없음.
///   F-02/F-03: native만 — FoT·무코드 영역 밖. receive()는 거부.
///   F-04: 모든 뷰·준비금 상계 상수 시간 — 사용자 수와 무관하다.
///   F-06: 요금·수취인 불변 — registrar성 권한 없음.
///   F-07: Subscribed 이벤트 ≠ 구독 상태 — 진실은 subscribers다.
contract SubscriptionManager is SimpleBrake, ReentrancyGuard {
    /// @custom:member expiry      만료 시각 (unix 초)
    /// @custom:member contributed 이번 이어달기(streak)에 낸 금액 —
    ///                           정산 전까지 환불 준비금을 구성한다
    struct Subscriber {
        uint64 expiry;
        uint88 contributed;
    }

    /// @dev 초당 요율 (wei) — 배포 시 영구 고정
    uint256 public immutable ratePerSecond;

    /// @dev 수익 수취 주소 — 고정. 키 행사가 아닌 지급 목적지다.
    address public immutable payee;

    mapping(address => Subscriber) public subscribers;

    /// @dev 활성 구독 납입 원금의 합(상계 상한). balance >= refundReserve
    ///      불변 — 환불 가능액은 항상 잔액으로 뒷받침된다.
    uint256 public refundReserve;

    error ZeroPayee();
    error PaymentTooSmall(uint256 paid, uint256 minimum);
    error NotSubscribed();
    error RefundFailed();
    error PayoutFailed();
    error NothingToSettle();
    error NothingToClaim();
    error AmountTooLarge();

    event Subscribed(address indexed user, uint256 paid, uint256 secondsAdded, uint64 newExpiry);
    event Cancelled(address indexed user, uint256 refund, uint64 oldExpiry);
    event Settled(address indexed user, uint256 amount);
    event RevenueClaimed(address payee, uint256 amount);

    constructor(address guardian, address payee_, uint256 ratePerSecond_) SimpleBrake(guardian) {
        if (payee_ == address(0)) revert ZeroPayee();
        if (ratePerSecond_ == 0) revert PaymentTooSmall(0, 1);
        payee = payee_;
        ratePerSecond = ratePerSecond_;
    }

    /// @dev 직접 송금 거부 — 기간 매수는 subscribe의 회계를 거친다.
    receive() external payable {
        revert PaymentTooSmall(0, ratePerSecond);
    }

    // ---------------------------------------------------------------- 구독

    /// @notice 보낸 금액만큼 구독 기간을 연장한다 (진입 — brake 차단).
    ///         미구독이면 지금부터, 구독 중이면 만료 시각에 이어서.
    ///         이전 기간이 이미 만료였다면 먼저 수익으로 정산한다.
    function subscribe() external payable nonReentrant whenEntryOpen {
        uint256 secondsToAdd = msg.value / ratePerSecond;
        if (secondsToAdd == 0) revert PaymentTooSmall(msg.value, ratePerSecond);

        Subscriber storage s = subscribers[msg.sender];
        if (s.expiry <= block.timestamp) _settle(msg.sender, s); // 죽은 기간 정산

        // 기존 만료가 미래면 이어서, 과거(또는 없으면)면 지금부터
        uint64 base = s.expiry > block.timestamp ? s.expiry : uint64(block.timestamp);
        uint256 value = secondsToAdd * ratePerSecond;
        // Both packed fields must represent the complete payment. Solidity's
        // narrowing casts silently discard high bits; truncating contributed
        // while increasing refundReserve would leave unreleasable principal.
        if (secondsToAdd > uint256(type(uint64).max) - base || value > uint256(type(uint88).max) - s.contributed) {
            revert AmountTooLarge();
        }
        uint64 newExpiry = base + uint64(secondsToAdd);

        s.expiry = newExpiry;
        s.contributed += uint88(value);
        refundReserve += value; // CEI: 스토리지 먼저
        emit Subscribed(msg.sender, msg.value, secondsToAdd, newExpiry);
    }

    /// @notice 남은 기간을 비례 환불받고 구독을 끊는다 (탈출 — brake 무관).
    ///         납입 원금 전액이 준비금에서 해제된다 — 소비분은 이 순간
    ///         수익이 된다.
    function cancel() external nonReentrant {
        Subscriber storage s = subscribers[msg.sender];
        uint256 remaining = s.expiry > block.timestamp ? s.expiry - block.timestamp : 0;
        if (remaining == 0) revert NotSubscribed();

        uint64 oldExpiry = s.expiry;
        uint256 refund = remaining * ratePerSecond;

        s.expiry = uint64(block.timestamp); // CEI: 먼저 끊는다
        uint256 contributed = s.contributed;
        s.contributed = 0;
        refundReserve -= contributed; // 납입 전액 해제

        (bool ok,) = msg.sender.call{value: refund}("");
        if (!ok) revert RefundFailed(); // 환불 실패 — 상태는 롤백된다
        emit Cancelled(msg.sender, refund, oldExpiry);
    }

    // ---------------------------------------------------------------- 정산 (탈출 — brake 무관)

    /// @notice 만료된 구독의 납입 잔고를 수익으로 확정한다.
    ///         누구나 호출 가능 — 보상을 붙이고 싶은 키퍼의 기본 재료.
    function settleExpired(address user) external nonReentrant {
        Subscriber storage s = subscribers[user];
        if (s.expiry > block.timestamp || s.contributed == 0) revert NothingToSettle();
        _settle(user, s);
    }

    /// @notice 준비금으로 뒷받침되지 않는 잔액(=소비 확정 수익 + dust)을
    ///         payee에게 인도한다. 누구나 호출 가능 — payee는 온체인 행동이
    ///         필요 없다.
    function claimRevenue() external nonReentrant {
        uint256 amount = address(this).balance - refundReserve;
        if (amount == 0) revert NothingToClaim();

        (bool ok,) = payee.call{value: amount}("");
        if (!ok) revert PayoutFailed();
        emit RevenueClaimed(payee, amount);
    }

    // ---------------------------------------------------------------- 뷰 (F-04)

    function isSubscribed(address user) external view returns (bool) {
        return subscribers[user].expiry > block.timestamp;
    }

    /// @dev 현재 인도 가능한 수익 — 잔액에서 환불 준비금을 뺀 값.
    function claimableRevenue() external view returns (uint256) {
        return address(this).balance - refundReserve;
    }

    // ---------------------------------------------------------------- 내부

    /// @dev 납입 잔고를 준비금에서 해제해 수익으로 확정한다.
    function _settle(address user, Subscriber storage s) private {
        uint256 value = s.contributed;
        if (value == 0) return; // 이미 정산된 빈 기록 — 조용히 넘어간다

        s.contributed = 0;
        refundReserve -= value;
        emit Settled(user, value);
    }
}
