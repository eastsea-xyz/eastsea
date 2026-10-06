// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {SimpleBrake} from "src/system/SimpleBrake.sol";
import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";

/// @title 예제 17 — 에이전트 자판기
/// @notice 정가 선결제로 작업을 주문하고, 에이전트가 기한 내 결과물
///         해시를 등록하면 즉시 정산된다. 기한 내 등록이 없으면
///         주문자가 전액을 돌려받는다.
/// @dev
///  설계 결정 — 자판기는 심사하지 않는다:
///   - 에스크로(예제 9)는 구매자의 승인이 정산 조건이다. 이 예제는
///     그 단계를 없앤다: deliver 는 결과물 **해시** 등록만으로
///     정산한다. 온체인이 아는 것은 "기한 내 뭔가 등록됐는가"뿐이다.
///     품질 평가는 온체인 밖(평판·후기)의 영역 — 결과의 서사는
///     Delivered 로그에 있고 진실은 슬롯 부재다 (F-07).
///   - 결과물 자체(문자열·URL)는 상태가 아니다. 슬롯은
///     {buyer, due} 팩 하나 — 존재 = 진행 중. 배달·환불 모두 슬롯을
///     반납한다: 완결된 주문은 존재하지 않는다 (예제 16 원리).
///   - 명세(spec)와 결과(result)는 해시로만 다닌다. 주문이 무엇을
///     원했는지는 Ordered 로그의 specHash, 무엇이 왔는지는
///     Delivered 로그의 resultHash — 계약의 양 끝이 로그에 있고
///     슬롯은 진행 중인 돈만 추적한다.
///
///  완결 보장 — 누구도 기다리게 하지 않는다:
///   - deliver: 에이전트가 기한 내 등록하면 즉시 정산 (환불 불가).
///   - refund: 기한 후 주문자가 누르면 전액 회수 — 에이전트가
///     잠적해도 돈은 갇히지 않는다. 구매자 승인 대기 상태가 없으므로
///     "합의를 기다리는 돈"이라는 상태 자체가 존재하지 않는다.
///
///  F 매핑:
///   F-01: deliver/refund CEI — 슬롯 delete·이벤트가 전송보다 앞
///         + nonReentrant.
///   F-02/F-03: native만. receive() 거부 — 지불은 order(specHash)뿐.
///   F-04: 순회 없음 — 주문은 id 직접 조회, 통계는 이벤트 스캔.
///   F-05: 7702 위임 계정의 주문·환불 동일 — msg.sender 주소와
///         msg.value 비교뿐. 주문자가 어떤 코드를 깔았는지 무관.
///   F-06: agent 는 정산 수취인일 뿐 — 가격·기한은 immutable,
///         재고·상품 관리 키가 없다 (디지털 작업은 재고가 없다).
///   F-07: Delivered 이벤트 ≠ 품질 — 온체인 진실은 슬롯 부재와
///         정산 사실뿐. 이 예제의 주제다.
///   F-08: 무작위성 없음.
///
///  brake: order(진입 — 새 지불 약정)만 차단. 배달·환불(탈출)은
///  항상 개방 — 판매를 멈춰도 진행 중인 돈의 출구는 막지 않는다.
contract AgentVending is SimpleBrake, ReentrancyGuard {
    /// @dev 정산 수취인 (작업 수행 에이전트) — 배포 시 고정
    address public immutable agent;

    /// @dev 주문 정가 — 모든 주문 동일 (자판기)
    uint256 public immutable price;

    /// @dev 주문부터 배달 마감까지의 기한
    uint48 public immutable fulfilWindow;

    /// @dev 다음 주문 번호 — 1부터
    uint256 public orderCount;

    /// @dev 진행 중 주문 — 존재(buyer != 0) = 미배달·미환불
    struct Order {
        address buyer; // 환불 수령인
        uint48 due; // 배달 마감
    }
    mapping(uint256 => Order) public orders;

    error ZeroPrice();
    error ZeroWindow();
    error WrongPrice(uint256 paid, uint256 price);
    error UnknownOrder(uint256 id);
    error NotAgent(address caller);
    error NotBuyer(address caller);
    error DeliveryTooLate(uint256 at, uint256 due);
    error StillOpen(uint256 at, uint256 due);
    error PaymentNotAllowed();
    error PayoutFailed();

    event Ordered(uint256 indexed id, address indexed buyer, bytes32 specHash, uint48 due);
    event Delivered(uint256 indexed id, bytes32 resultHash);
    event Refunded(uint256 indexed id);

    /// @param guardian      비상 제동(신규 주문 차단만)
    /// @param agent_        정산 수취인
    /// @param price_        주문 정가
    /// @param fulfilWindow_ 배달 기한 (주문 시각부터)
    constructor(address guardian, address agent_, uint256 price_, uint48 fulfilWindow_) SimpleBrake(guardian) {
        if (price_ == 0) revert ZeroPrice();
        if (fulfilWindow_ == 0) revert ZeroWindow();
        if (agent_ == address(0)) revert NotAgent(address(0));

        agent = agent_;
        price = price_;
        fulfilWindow = fulfilWindow_;
    }

    /// @dev 지불은 order(specHash) 로만 — 밖 송금 거부
    receive() external payable {
        revert PaymentNotAllowed();
    }

    // ---------------------------------------------------------------- 주문 (진입 — brake 차단)

    /// @notice 정가로 작업을 주문한다. specHash 는 주문 명세의
    ///         커밋(해시) — 로그에만 남는다.
    function order(bytes32 specHash) external payable whenEntryOpen returns (uint256 id) {
        if (msg.value != price) revert WrongPrice(msg.value, price);

        id = ++orderCount;
        orders[id] = Order({buyer: msg.sender, due: uint48(block.timestamp) + fulfilWindow});
        emit Ordered(id, msg.sender, specHash, uint48(block.timestamp) + fulfilWindow);
    }

    // ---------------------------------------------------------------- 배달 (탈출 — 슬롯 반납·정산)

    /// @notice 기한 내 결과물 해시를 등록해 정산받는다 — agent 전용.
    ///         심사는 없다: 등록이 곧 정산이다.
    function deliver(uint256 id, bytes32 resultHash) external nonReentrant {
        if (msg.sender != agent) revert NotAgent(msg.sender);
        Order memory o = orders[id];
        if (o.buyer == address(0)) revert UnknownOrder(id);
        if (block.timestamp > o.due) revert DeliveryTooLate(block.timestamp, o.due);

        delete orders[id]; // CEI: 반납이 전송보다 먼저 — 재진입해도 주문이 없다
        emit Delivered(id, resultHash);

        (bool ok,) = agent.call{value: price}("");
        if (!ok) revert PayoutFailed();
    }

    // ---------------------------------------------------------------- 환불 (탈출 — 슬롯 반납)

    /// @notice 기한이 지나도 배달이 없으면 전액을 돌려받는다 —
    ///         주문자 전용. 누구도 잠적한 에이전트를 기다리지 않는다.
    function refund(uint256 id) external nonReentrant {
        Order memory o = orders[id];
        if (o.buyer == address(0)) revert UnknownOrder(id);
        if (msg.sender != o.buyer) revert NotBuyer(msg.sender);
        if (block.timestamp <= o.due) revert StillOpen(block.timestamp, o.due);

        delete orders[id]; // CEI
        emit Refunded(id);

        (bool ok,) = o.buyer.call{value: price}("");
        if (!ok) revert PayoutFailed();
    }
}
