// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "openzeppelin/token/ERC20/IERC20.sol";
import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";
import {SafeToken} from "src/common/SafeToken.sol";

/// @title 예제 7-B — 자립형 선형 베스팅 (단일 그랜트)
/// @notice 배포 시 그랜트 전액을 예치하고 그대로 방치하는 1회성 베스팅.
///         운영자·회수·관리자가 없다 — 컨트랙트가 곧 그랜트다.
///         (투자자 어카운트, 창업자 보유분 잠금 등 1인용 인스턴스)
/// @dev
///  TokenTimeLock(배치 관리형)과의 대비:
///   - 사용자 상태가 claimed 1슬롯뿐이다. 초기 설계는 이마저 없애
///     claimed = total - token.balanceOf(this) 로 잔액에서 유도했으나,
///     제3자가 토큰을 기부해 잔액이 total을 넘기는 순간 지급누적이
///     0으로 포화되어 total 초과 지급이 가능해진다 (PoC:
///     test_vesting_toleratesDonation). 정확성을 위해 1슬롯을 택했다.
///   - brake가 없다: 어차피 claim(탈출)만 있고 진입이 없어 brake가
///     막을 것이 없다. 배치 컨트랙트에만 brake가 의미 있다.
///   - revoke 없음 — 그랜트는 비가역이다. 되돌릴 주체도 없다.
///
///  F 매핑:
///   F-01: claim nonReentrant + CEI(claimed 갱신 후 push).
///   F-02/F-03: SafeToken 경로, 생성자 무코드 거부.
///   F-04: vested/claimable 상수 시간.
contract LinearVesting is ReentrancyGuard {
    uint256 private constant _MAX_UINT128 = type(uint128).max;

    IERC20 public immutable token;
    address public immutable beneficiary;
    uint128 public immutable total;
    uint48 public immutable start;
    uint48 public immutable cliff;
    uint48 public immutable duration;

    /// @dev 지급누적 — claim에서만 증가, vested 이하로 단조 증가한다
    uint128 private claimed;

    error ZeroAmount();
    error ZeroDuration();
    error AmountTooLarge(uint256 amount);

    event Claimed(address indexed beneficiary, uint256 amount);

    constructor(IERC20 token_, address beneficiary_, uint256 amount, uint256 cliffSec, uint256 durationSec) {
        if (address(token_).code.length == 0) revert ZeroAmount(); // F-03
        if (beneficiary_ == address(0)) revert ZeroAmount();
        if (amount == 0) revert ZeroAmount();
        if (amount > _MAX_UINT128) revert AmountTooLarge(amount);
        if (durationSec == 0 || cliffSec > durationSec) revert ZeroDuration();

        token = token_;
        beneficiary = beneficiary_;
        total = uint128(amount);
        start = uint48(block.timestamp);
        cliff = uint48(cliffSec);
        duration = uint48(durationSec);

        SafeToken.pullExact(token_, msg.sender, amount); // 그랜트 전액 예치
    }

    /// @notice 수혜자에게 해제분을 지급한다. 누구나 호출할 수 있다
    ///         (가스 대낭) — 수령인은 언제나 beneficiary다.
    function claim() external nonReentrant returns (uint256 paid) {
        uint256 v = _vested();
        paid = v > claimed ? v - claimed : 0;
        if (paid > 0) {
            claimed = uint128(claimed + paid); // CEI: 갱신 먼저
            SafeToken.push(token, beneficiary, paid);
        }
        emit Claimed(beneficiary, paid);
    }

    // ---------------------------------------------------------------- 뷰 (F-04)

    function vested() external view returns (uint256) {
        return _vested();
    }

    /// @notice 지금 claim하면 나오는 양 = vested - 지급누적.
    function claimable() external view returns (uint256) {
        uint256 v = _vested();
        return v > claimed ? v - claimed : 0;
    }

    function _vested() private view returns (uint256) {
        uint256 elapsed = block.timestamp - start;
        if (elapsed <= cliff) return 0;
        if (elapsed >= duration) return total;
        return (total * (elapsed - cliff)) / (duration - cliff);
    }
}
