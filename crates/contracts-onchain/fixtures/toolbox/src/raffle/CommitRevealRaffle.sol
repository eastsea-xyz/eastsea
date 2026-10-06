// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";
import {SimpleBrake} from "src/system/SimpleBrake.sol";

/// @title 예제 14 — 커밋-리빌 래플
/// @notice 참가비 풀 추첨. 주최자는 참가 모집 전에 시드를 해시로
///         커밋하고, 참가 마감 후 공개한다. 우승 인덱스는 시드와
///         블록 prevrandao를 섞어 뽑는다.
/// @dev
///  설계 결정 — 무작위성은 두 주체의 합작이어야 한다 (F-08):
///   - prevrandao 단독: 블록 제안 검증자가 결과를 조작할 수 있다.
///   - operator 시드 단독: operator가 미리 시뮬레이션하고 유리한
///     시드만 공개할 수 있다.
///   - 이 예제: winner = players[hash(seed ⊕ prevrandao) % N].
///     operator는 참가자 명단이 확정되기 **전에** 시드를 커밋하고
///     (keccak(seed)만 배포 시 고정), 검증자는 시드를 모른다.
///     어느 한쪽도 단독으로 결과를 고를 수 없다.
///
///  리빌 거부(withholding)의 완결 보장:
///   - operator가 불리한 결과를 보고 리빌을 거부할 수 있다.
///   - 리빌 마감 후 누구나 drawWithoutSeed()로 prevrandao만으로
///     추첨을 강행한다 — operator의 시드는 버려지고 참가비는
///     반드시 참가자에게 간다. "안 뽑힌 채로 굶는" 상태가 없다.
///
///  1회성 라이프사이클:
///   - 참가(진입) → 리빌/추첨(판정) → 지급(탈출). 추첨은 되돌릴
///     수 없으므로 컨트랙트 자체를 1회용으로 쓴다 — 재추첨 키를
///     만드는 대신 새 라플을 새로 배포한다.
///   - 참가자는 배열에 push한다(티켓당 1슬롯). 중복 참가 = 여러
///     티켓. 우승자 판정은 인덱스 산수 1회 — 순회 없음(F-04).
///
///  F 매핑:
///   F-01: reveal/draw CEI(drawn 플래그 먼저, 상금 전송 마지막)
///         + nonReentrant.
///   F-02/F-03: native만. receive()는 참가비 밖 송금을 거부.
///   F-04: 참가자 순회·합계 뷰 없음 — length와 인덱스 산수만.
///   F-05: 7702 위임 EOA 참가·수령 동일 — 주소 비교뿐.
///   F-06: operator는 시드 보유자일 뿐 — 커밋·마감·참가비 전부
///         immutable로 박혀 교체·개입 불가.
///   F-07: Entered 이벤트 ≠ 참가 — 진실은 players 배열이다.
///   F-08: 위 혼합 설계가 이 예제의 주제다.
contract CommitRevealRaffle is SimpleBrake, ReentrancyGuard {
    /// @dev operator 시드의 해시 — keccak256(abi.encodePacked(seed))
    bytes32 public immutable seedCommit;

    /// @dev 참가 마감 — 이후 참가 불가, 리빌 창구가 열린다
    uint48 public immutable enterDeadline;

    /// @dev 리빌 마감 — 이후 drawWithoutSeed로 추첨 강행
    uint48 public immutable revealDeadline;

    /// @dev 티켓 1장 가격 (wei)
    uint256 public immutable ticketPrice;

    /// @dev 참가 순서 그대로 — 우승 인덱스의 원본
    address[] public players;

    /// @dev 추첨 완료 — 1회. 우승자 주소가 곧 증명이다
    address public winner;
    bool public drawn;

    error ZeroCommit();
    error ZeroPrice();
    error ZeroPeriod();
    error ZeroTickets();
    error WrongPrice(uint256 paid, uint256 price);
    error EntryClosed(uint256 at, uint256 until);
    error NoTickets();
    error RevealTooEarly(uint256 at, uint256 from);
    error RevealClosed(uint256 at, uint256 until);
    error InvalidSeed(bytes32 got, bytes32 want);
    error SeedWithheld(uint256 at, uint256 until);
    error AlreadyDrawn();
    error PayoutFailed();

    event Entered(address indexed player, uint256 indexed ticket, uint256 total);
    event Drawn(address indexed winner, uint256 prize, bytes32 mixedSeed);

    /// @param guardian      비상 제동(신규 참가 차단만)
    /// @param seedCommit_   keccak256(abi.encodePacked(seed)) — 참가 모집 전 커밋
    /// @param ticketPrice_  티켓 가격
    /// @param enterPeriod_  참가 기간 (배포 시각부터)
    /// @param revealWindow_ 리빌 창구 (참가 마감부터)
    constructor(address guardian, bytes32 seedCommit_, uint256 ticketPrice_, uint48 enterPeriod_, uint48 revealWindow_)
        SimpleBrake(guardian)
    {
        if (seedCommit_ == bytes32(0)) revert ZeroCommit();
        if (ticketPrice_ == 0) revert ZeroPrice();
        if (enterPeriod_ == 0 || revealWindow_ == 0) revert ZeroPeriod();

        seedCommit = seedCommit_;
        ticketPrice = ticketPrice_;
        enterDeadline = uint48(block.timestamp) + enterPeriod_;
        revealDeadline = uint48(block.timestamp) + enterPeriod_ + revealWindow_;
    }

    /// @dev 참가비 밖 송금 거부 — 풀은 티켓 판매로만 커진다
    receive() external payable {
        revert WrongPrice(msg.value, ticketPrice);
    }

    // ---------------------------------------------------------------- 참가 (진입 — brake 차단)

    /// @notice 티켓 1장을 산다. 여러 장은 여러 번 호출한다.
    function enter() external payable nonReentrant whenEntryOpen {
        if (msg.value != ticketPrice) revert WrongPrice(msg.value, ticketPrice);
        if (block.timestamp > enterDeadline) revert EntryClosed(block.timestamp, enterDeadline);

        players.push(msg.sender);
        emit Entered(msg.sender, players.length - 1, players.length);
    }

    // ---------------------------------------------------------------- 추첨 (판정)

    /// @notice 참가 마감 후 operator 시드를 공개해 추첨한다.
    ///         누구나 호출 가능 — 공개된 시드는 누구나 제출할 수 있다.
    function reveal(bytes32 seed) external nonReentrant {
        if (drawn) revert AlreadyDrawn();
        if (block.timestamp <= enterDeadline) revert RevealTooEarly(block.timestamp, enterDeadline);
        if (block.timestamp > revealDeadline) revert RevealClosed(block.timestamp, revealDeadline);

        bytes32 got = keccak256(abi.encodePacked(seed));
        if (got != seedCommit) revert InvalidSeed(got, seedCommit);

        uint256 n = players.length;
        if (n == 0) revert NoTickets();

        bytes32 mixed = keccak256(abi.encode(seed, block.prevrandao));
        _payout(players[uint256(mixed) % n], mixed);
    }

    /// @notice 리빌 마감 후 추첨을 강행한다 — operator가 시드를
    ///         보류하면 시드 없이 prevrandao만으로 뽑는다.
    function drawWithoutSeed() external nonReentrant {
        if (drawn) revert AlreadyDrawn();
        if (block.timestamp <= revealDeadline) revert SeedWithheld(block.timestamp, revealDeadline);

        uint256 n = players.length;
        if (n == 0) revert NoTickets();

        _payout(players[uint256(block.prevrandao) % n], bytes32(0));
    }

    // ---------------------------------------------------------------- 지급 (탈출 — brake 무관)

    /// @dev drawn 플래그를 먼저 올리고(CEI) 상금 전액을 보낸다.
    function _payout(address who, bytes32 mixed) private {
        drawn = true; // CEI: 두 번째 추첨은 죽는다
        winner = who;
        emit Drawn(who, address(this).balance, mixed);

        (bool ok,) = who.call{value: address(this).balance}("");
        if (!ok) revert PayoutFailed();
    }

    // ---------------------------------------------------------------- 뷰 (F-04)

    function playerCount() external view returns (uint256) {
        return players.length;
    }
}
