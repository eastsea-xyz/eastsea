// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";
import {SimpleBrake} from "src/system/SimpleBrake.sol";

/// @title 예제 9 — 마일스톤 에스크로
/// @notice 구매자가 native coin을 예치하고, 결과에 만족할 때마다
///         마일스톤 승인으로 판매자에게 인출권을 넘긴다. 미승인 잔여는
///         언제든 구매자가 회수한다.
/// @dev
///  설계 결정 — 상태를 마일스톤 배열에 두지 않는다:
///   - 마일스톤 금액 배열을 스토리지에 저장하지 않는다. 금액은
///     승인 시점에 구매자가 amount로 지정한다 — "승인된 누적"만
///     상태로 남는다 (approvedTotal). 배열 N개 대신 스칼라 1개.
///   - 인덱스별 중복 승인만 해시 슬롯으로 막는다:
///     milestoneApproved[keccak256(dealId, index)].
///   - 마일스톤 개수는 생성 시 고정 — 예치 시점의 합의 구조가
///     거래 중 바뀌지 않게 하는 가드일 뿐, 금액 목록은 오프체인
///     계약서가 진실이다.
///
///  자금 흐름:
///   deposited == approvedTotal + refundable — 불변식.
///   판매자 인출 가능 = approvedTotal - withdrawnTotal.
///   구매자 환불 가능 = deposited - approvedTotal (승인분은 못 되돌린다).
///
///  F 매핑:
///   F-01: 상태변경 함수 전부 nonReentrant + CEI(스토리지 먼저,
///         native 전송 마지막).
///   F-02/F-03: native만 다룬다 — FoT·무코드 문제 영역 밖.
///   F-04: 뷰 상수 시간.
///   F-07: ApproveMilestone 이벤트 ≠ 승인 — 진실은 상태다.
contract MilestoneEscrow is SimpleBrake, ReentrancyGuard {
    /// @param buyer            예치자
    /// @param seller           인출권자
    /// @param deposited        남은 예치 (환불 시 감소 — 처분 가능 원금)
    /// @param approvedTotal    승인 누적 (단조 증가, 취소 없음)
    /// @param withdrawnTotal   판매자 인출 누적
    /// @param milestoneCount   합의된 마일스톤 수 (중복 승인 가드 상한)
    struct Deal {
        address buyer;
        address seller;
        uint128 deposited;
        uint128 approvedTotal;
        uint128 withdrawnTotal;
        uint64 milestoneCount;
    }

    uint256 public nextDealId = 1; // 0은 미사용 — 존재 검사를 위해

    mapping(uint256 => Deal) public deals;

    /// @dev keccak256(dealId, index) → 승인 여부. 인덱스당 1회 승인.
    mapping(bytes32 => bool) public milestoneApproved;

    error NotDealParty();
    error DealNotFound(uint256 dealId);
    error ZeroAmount();
    error ZeroMilestones();
    error ExceedsDeposit(uint256 wanted, uint256 refundable);
    error MilestoneAlreadyApproved(uint256 dealId, uint64 index);
    error IndexOutOfRange(uint64 index, uint64 count);
    error NothingToWithdraw();
    error NothingToRefund();
    error NativeTransferFailed();

    event DealCreated(
        uint256 indexed dealId, address indexed buyer, address indexed seller, uint256 amount, uint64 milestones
    );
    event MilestoneApproved(uint256 indexed dealId, uint64 index, uint256 amount);
    event SellerWithdrawn(uint256 indexed dealId, uint256 amount);
    event BuyerRefunded(uint256 indexed dealId, uint256 amount);

    constructor(address guardian) SimpleBrake(guardian) {}

    /// @dev 직접 송금 거부 — 자금은 createDeal의 회계를 거쳐야 한다.
    receive() external payable {
        revert NativeTransferFailed();
    }

    // ---------------------------------------------------------------- 거래

    /// @notice 에스크로를 열고 전액을 예치한다 (진입 — brake 차단).
    ///         마일스톤 금액 목록은 오프체인 계약서에 두고, 여기는
    ///         개수만 고정한다.
    function createDeal(address seller, uint64 milestoneCount) external payable nonReentrant whenEntryOpen {
        if (msg.value == 0) revert ZeroAmount();
        if (seller == address(0) || seller == msg.sender) revert NotDealParty();
        if (milestoneCount == 0) revert ZeroMilestones();

        uint256 dealId = nextDealId;
        nextDealId = dealId + 1;
        deals[dealId] = Deal(msg.sender, seller, uint128(msg.value), 0, 0, milestoneCount);
        emit DealCreated(dealId, msg.sender, seller, msg.value, milestoneCount);
    }

    /// @notice 마일스톤 승인 — 이후 그 금액은 판매자 몫이 된다.
    ///         승인은 취소할 수 없다 (되돌림은 별도 합의·환불 절차).
    function approveMilestone(uint256 dealId, uint64 index, uint256 amount) external nonReentrant {
        Deal storage d = deals[dealId];
        if (d.deposited == 0) revert DealNotFound(dealId);
        if (msg.sender != d.buyer) revert NotDealParty();
        if (amount == 0) revert ZeroAmount();
        if (index >= d.milestoneCount) revert IndexOutOfRange(index, d.milestoneCount);

        bytes32 key = keccak256(abi.encode(dealId, index));
        if (milestoneApproved[key]) revert MilestoneAlreadyApproved(dealId, index);

        uint256 refundable = uint256(d.deposited) - d.approvedTotal;
        if (amount > refundable) revert ExceedsDeposit(amount, refundable);

        milestoneApproved[key] = true;
        d.approvedTotal += uint128(amount); // 승인 즉시 판매자 귀속
        emit MilestoneApproved(dealId, index, amount);
    }

    // ---------------------------------------------------------------- 인출 (탈출 — brake 무관)

    /// @notice 승인 누적 중 아직 인출하지 않은 분을 판매자가 받는다.
    function sellerWithdraw(uint256 dealId) external nonReentrant {
        Deal storage d = deals[dealId];
        if (d.deposited == 0) revert DealNotFound(dealId);
        if (msg.sender != d.seller) revert NotDealParty();

        uint256 due = uint256(d.approvedTotal) - d.withdrawnTotal;
        if (due == 0) revert NothingToWithdraw();

        d.withdrawnTotal = d.approvedTotal; // CEI: 스토리지 먼저
        _pay(d.seller, due);
        emit SellerWithdrawn(dealId, due);
    }

    /// @notice 미승인 잔여를 구매자가 회수한다. 승인분은 유지된다.
    function buyerRefund(uint256 dealId) external nonReentrant {
        Deal storage d = deals[dealId];
        if (d.deposited == 0) revert DealNotFound(dealId);
        if (msg.sender != d.buyer) revert NotDealParty();

        uint256 refundable = uint256(d.deposited) - d.approvedTotal;
        if (refundable == 0) revert NothingToRefund();

        d.deposited = d.approvedTotal; // 회수분 제거 — 잔여는 승인분뿐
        _pay(d.buyer, refundable);
        emit BuyerRefunded(dealId, refundable);
    }

    // ---------------------------------------------------------------- 뷰 (F-04)

    function dealInfo(uint256 dealId)
        external
        view
        returns (address buyer, address seller, uint256 refundable, uint256 withdrawable)
    {
        Deal storage d = deals[dealId];
        return (d.buyer, d.seller, uint256(d.deposited) - d.approvedTotal, uint256(d.approvedTotal) - d.withdrawnTotal);
    }

    // ---------------------------------------------------------------- 내부

    function _pay(address to, uint256 amount) private {
        (bool ok,) = to.call{value: amount}("");
        if (!ok) revert NativeTransferFailed();
    }
}
