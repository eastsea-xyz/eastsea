// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "openzeppelin/token/ERC20/IERC20.sol";
import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";
import {SimpleBrake} from "src/system/SimpleBrake.sol";
import {SafeToken} from "src/common/SafeToken.sol";

/// @title 예제 7-A — 배치형 토큰 타임락
/// @notice 누구나 토큰을 예치하고 수혜자의 클리프·기간을 지정해 잠근다.
///         수혜자는 cliff 후 선형 해제분을 release()로 인출한다.
///         운영자 키가 없다 — 예치자 본인이 자금을 넣고 파라미터를 정한다.
/// @dev
///  설계 결정:
///   - 수혜자당 활성 잠금 1개(AlreadyLocked). 병합은 cliff/duration이
///     섞이는 회계를 만들므로 금지 — 다음 그랜트는 인출 후 가능.
///   - released를 잔액이 아닌 슬롯에 유지한다: 배치 컨트랙트의 잔액은
///     여러 수혜자가 공유해 누가 얼마나 뺐는지 잔액만으로 알 수 없다.
///   - Lock 1개를 2슬롯(uint128+uint128 / uint48x3)에 팩한다.
///
///  F 매핑:
///   F-01: lock/release 모두 nonReentrant + CEI(스냅샷 후 push).
///   F-02: 예치는 pullExact — 정확한 금액이 잠금 계약의 근거다.
///   F-03: 생성자 무코드 거부, 이동은 SafeToken.
///   F-04: vested/releasable 상수 시간.
contract TokenTimeLock is SimpleBrake, ReentrancyGuard {
    uint256 private constant _MAX_UINT128 = type(uint128).max;

    IERC20 public immutable token;

    /// @param amount   잠긴 총액
    /// @param released 지금까지 지급된 누적
    /// @param start    lock 시각 (초)
    /// @param cliff    start 이후 클리프까지 상대 초
    /// @param duration start 이후 전량 해제까지 상대 초 (cliff 포함, >= cliff)
    struct Lock {
        uint128 amount;
        uint128 released;
        uint48 start;
        uint48 cliff;
        uint48 duration;
    }

    mapping(address => Lock) public locks;

    error ZeroAmount();
    error ZeroDuration();
    error AmountTooLarge(uint256 amount);
    error AlreadyLocked(address beneficiary);
    error NothingToRelease();

    event Locked(
        address indexed depositor, address indexed beneficiary, uint256 amount, uint256 cliff, uint256 duration
    );
    event Released(address indexed beneficiary, uint256 amount);

    constructor(address guardian, IERC20 token_) SimpleBrake(guardian) {
        // F-03: 무코드 주소는 배포 단계에서 거부한다.
        if (address(token_).code.length == 0) revert ZeroAmount();
        token = token_;
    }

    // ---------------------------------------------------------------- 잠금

    /// @notice 토큰을 예치해 수혜자의 잠금을 만든다 (진입 — brake 차단).
    ///         수혜자당 활성 잠금은 1개다.
    function lockFor(address beneficiary, uint256 amount, uint256 cliffSec, uint256 durationSec)
        external
        nonReentrant
        whenEntryOpen
    {
        if (amount == 0) revert ZeroAmount();
        if (durationSec == 0 || cliffSec > durationSec) revert ZeroDuration();
        if (amount > _MAX_UINT128) revert AmountTooLarge(amount);
        Lock storage existing = locks[beneficiary];
        if (existing.amount != 0 && existing.released < existing.amount) revert AlreadyLocked(beneficiary);

        SafeToken.pullExact(token, msg.sender, amount); // F-02: 정확한 금액
        locks[beneficiary] = Lock(uint128(amount), 0, uint48(block.timestamp), uint48(cliffSec), uint48(durationSec));
        emit Locked(msg.sender, beneficiary, amount, cliffSec, durationSec);
    }

    /// @notice 해제분을 인출한다 (탈출 — brake와 무관하게 항상 열려 있다).
    ///         수혜자 본인만 호출한다.
    function release() external nonReentrant {
        Lock storage l = locks[msg.sender];
        // vested는 단조 증가, released는 vested에서만 나가므로 차 >= 0
        uint256 due = _vested(msg.sender) - l.released;
        if (due == 0) revert NothingToRelease();
        l.released = uint128(l.released + due); // CEI: 스냅샷 먼저
        SafeToken.push(token, msg.sender, due);
        emit Released(msg.sender, due);
    }

    // ---------------------------------------------------------------- 뷰 (F-04)

    /// @notice t=현재까지의 선형 해제 누적 (cliff 전 0).
    function vested(address beneficiary) external view returns (uint256) {
        return _vested(beneficiary);
    }

    /// @notice 지금 release()하면 나오는 양.
    function releasable(address beneficiary) external view returns (uint256) {
        Lock storage l = locks[beneficiary];
        if (l.amount == 0) return 0;
        return _vested(beneficiary) - l.released;
    }

    function _vested(address beneficiary) private view returns (uint256) {
        Lock storage l = locks[beneficiary];
        if (l.amount == 0) return 0;
        uint256 elapsed = block.timestamp - l.start;
        if (elapsed <= l.cliff) return 0; // 클리프 전: 0 (경계 포함)
        if (elapsed >= l.duration) return l.amount;
        return (uint256(l.amount) * (elapsed - l.cliff)) / (uint256(l.duration) - l.cliff);
    }
}
