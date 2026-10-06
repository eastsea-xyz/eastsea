// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SimpleBrake} from "src/system/SimpleBrake.sol";
import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";

/// @title 예제 16 — 인보이스 장부
/// @notice 수취인(payee)이 고정액 청구서를 발행하고 고객이 정확한
///         금액을 결제하면 즉시 정산된다. 결제·폐기된 청구서는
///         슬롯을 통째로 반납한다.
/// @dev
///  설계 결정 — 청구서는 기다리는 상태다:
///   - 인보이스 슬롯의 수명은 "발행 → 결제/폐기"까지다. settle 은
///     매핑 원소를 delete 한 뒤 전송한다(CEI). 결제된 청구서는
///     존재하지 않으므로 이중결제가 원천 차단된다 — paid 플래그
///     같은 반쪽 상태("결제됐지만 슬롯은 남는")를 만들지 않는다.
///   - 영수증은 이벤트(Settled)와 누계 스칼라(totalSettled)다.
///     "이 청구서 결제됐나?"의 온체인 답은 "그런 청구서가 없다" —
///     증빙은 로그 스캔, 진실은 부재로 표현된다 (F-07 의 역발상).
///   - 메모(품목·주문번호)는 스토리지가 아니라 Issued 로그의
///     페이로드다. 영구 상태는 amount+due 팩 1슬롯뿐.
///
///  만료는 가격 보장의 종료지 결제 권리의 종료가 아니다:
///   - due 이후 settle 은 거부(InvoiceExpired) — 폐기된 가격으로
///     결제를 강제받는 일이 없다. 만료된 청구서는 누구나 purge 로
///     슬롯을 반납할 수 있다(상태 위생).
///   - void 는 payee 전용 폐기 — 만료 전이라도 내 청구서는 내 마음.
///
///  F 매핑:
///   F-01: settle CEI — delete·누계·이벤트가 전송보다 전부 앞.
///   F-02/F-03: native만. receive() 거부 — 결제는 settle(id)뿐.
///   F-04: 순회 없음 — 청구서는 id 직접 조회, 누계는 스칼라.
///   F-05: 7702 위임 EOA 결제 동일 — msg.value 비교뿐.
///   F-06: payee 는 수취인일 뿐 — 발행 권한 외에 아무 키가 없다
///         (재발행 키, 금액 수정 키 없음. 잘못 냈으면 새 청구서).
///   F-07: Settled 이벤트 ≠ 정산 — 진실은 totalSettled 와 슬롯 부재.
///   F-08: 무작위성 없음.
///
///  brake: issue(진입 — 새 청구 권리 생성)만 차단한다. 결제·폐기·
///  반납(탈출)은 항상 개방 — 장부를 멈춰도 이미 발행된 돈의 흐름은
///  막지 않는다.
contract InvoiceBook is SimpleBrake, ReentrancyGuard {
    /// @dev 정산 수취인 — 배포 시 고정
    address public immutable payee;

    /// @dev 다음 청구서 번호 — 1부터
    uint256 public invoiceCount;

    /// @dev 청구서 — 존재(amount != 0) = 미결제. 슬롯 1개에 팩.
    struct Invoice {
        uint128 amount; // 청구 금액
        uint48 due; // 결제 마감
    }
    mapping(uint256 => Invoice) public invoices;

    /// @dev 정산 누계 — payee 의 온체인 실적 스칼라 (평판 신호)
    uint256 public totalSettled;

    error ZeroAmount();
    error ZeroPeriod();
    error NotPayee(address caller);
    error UnknownInvoice(uint256 id);
    error WrongAmount(uint256 paid, uint256 want);
    error InvoiceExpired(uint256 at, uint256 due);
    error NotPastDue(uint256 at, uint256 due);
    error PaymentNotAllowed();
    error SettleTransferFailed();

    event Issued(uint256 indexed id, uint256 amount, uint48 due, string memo);
    event Settled(uint256 indexed id, address indexed payer, uint256 amount);
    event Voided(uint256 indexed id);
    event Purged(uint256 indexed id, address indexed by);

    /// @param guardian 비상 제동(신규 발행 차단만)
    /// @param payee_    정산 수취인
    constructor(address guardian, address payee_) SimpleBrake(guardian) {
        if (payee_ == address(0)) revert NotPayee(address(0));
        payee = payee_;
    }

    /// @dev 결제는 settle(id) 로만 — 밖 송금 거부
    receive() external payable {
        revert PaymentNotAllowed();
    }

    // ---------------------------------------------------------------- 발행 (진입 — brake 차단)

    /// @notice 청구서를 발행한다. memo(품목·주문번호)는 로그에만 남는다.
    function issue(uint128 amount, uint48 payBy, string calldata memo) external whenEntryOpen returns (uint256 id) {
        if (msg.sender != payee) revert NotPayee(msg.sender);
        if (amount == 0) revert ZeroAmount();
        if (payBy == 0) revert ZeroPeriod();

        id = ++invoiceCount;
        invoices[id] = Invoice({amount: amount, due: uint48(block.timestamp) + payBy});
        emit Issued(id, amount, uint48(block.timestamp) + payBy, memo);
    }

    // ---------------------------------------------------------------- 결제 (탈출 — 슬롯 반납)

    /// @notice 청구서를 정확한 금액으로 결제한다. 정산은 즉시.
    function settle(uint256 id) external payable nonReentrant {
        Invoice memory inv = invoices[id]; // 결제 후 부재가 답이다
        if (inv.amount == 0) revert UnknownInvoice(id);
        if (msg.value != inv.amount) revert WrongAmount(msg.value, inv.amount);
        if (block.timestamp > inv.due) revert InvoiceExpired(block.timestamp, inv.due);

        delete invoices[id]; // CEI: 슬롯 반납이 전송보다 먼저
        totalSettled += inv.amount;
        emit Settled(id, msg.sender, inv.amount);

        (bool ok,) = payee.call{value: inv.amount}("");
        if (!ok) revert SettleTransferFailed();
    }

    // ---------------------------------------------------------------- 폐기·반납 (탈출)

    /// @notice 미결제 청구서를 폐기한다 — payee 전용, 만료 무관.
    function void(uint256 id) external {
        if (msg.sender != payee) revert NotPayee(msg.sender);
        if (invoices[id].amount == 0) revert UnknownInvoice(id);

        delete invoices[id];
        emit Voided(id);
    }

    /// @notice 만료된 미결제 청구서의 슬롯을 반납한다. 누구나.
    function purge(uint256 id) external {
        Invoice memory inv = invoices[id];
        if (inv.amount == 0) revert UnknownInvoice(id);
        if (block.timestamp <= inv.due) revert NotPastDue(block.timestamp, inv.due);

        delete invoices[id];
        emit Purged(id, msg.sender);
    }
}
