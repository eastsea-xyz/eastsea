// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "openzeppelin/token/ERC20/IERC20.sol";
import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";
import {SimpleBrake} from "src/system/SimpleBrake.sol";
import {SafeToken} from "src/common/SafeToken.sol";

/// @title 예제 6 — 스테이킹 보상 분배기
/// @notice ERC-20을 스테이크하면 보상 토큰을 시간 비례로 분배한다.
///         후원자가 fundRewards(amount, duration)로 보상 풀을 채우면
///         duration 동안 선형 방출된다.
/// @dev
///  회계 — 잔액 기반(유료 상태 철학: 잔액이 진실):
///   totalDebt = 사용자에게 귀속됐지만 아직 지급되지 않은 총액.
///   rewardToken 잔액 - totalDebt = 아직 방출되지 않은 미래분.
///   다음 fund는 이 "남은 분"을 새 기간에 다시 반영한다 — 스테이커가
///   아무도 없던 구간의 방출 예정분이 소실되지 않는다.
///
///  분배 수학 (Synthetix StakingRewards 형식 + 함정 제거):
///   rewardPerTokenStored += elapsed * rate * 1e18 / totalStaked
///   earned(user) = userOwed + staked * (rewardPerToken - userRewardPaid) / 1e18
///   Synthetix 원본은 전액 인출 시 staked=0이 되며 미청구 보상이 소실된다
///   (유명한 함정). 여기는 checkpoint 시점마다 진행분을 userOwed로 이월해
///   unstake 이후에도 보상이 살아 있다.
///
///  불변식:
///   I1 balance(reward) == totalDebt + 미방출분 — 결코 부족 지급하지 않는다
///   I2 totalDebt == Sum(earned) — 전원 claim이 잔액을 전부 소진한다
///
///  F 매핑:
///   F-01: stake/unstake/claim/fund 전부 nonReentrant + CEI(스냅샷 후 push).
///   F-02: 스테이크 유입은 pull의 도착량 기준 — FoT 스테이크 토큰도
///         실제 도착량만 크레딧된다 (좌초 없음).
///   F-03: 토큰 이동은 SafeToken(무코드 거부 + strict bool).
///   F-04: 뷰는 상수 시간.
contract RewardDistributor is SimpleBrake, ReentrancyGuard {
    uint256 private constant _PRECISION = 1e18;

    IERC20 public immutable stakingToken;
    IERC20 public immutable rewardToken;

    uint256 public totalStaked;
    /// @dev 누적 보상/토큰 (1e18 스케일)
    uint256 public rewardPerTokenStored;
    /// @dev 귀속 미지급 총액 — balance - totalDebt가 미방출분
    uint256 public totalDebt;
    /// @dev 초당 보상 (finish 전까지)
    uint256 public rewardRate;
    uint256 public finishAt;
    uint256 public updatedAt;

    mapping(address => uint256) public userStaked;
    /// @dev 사용자별 마지막 상호작용 시점의 rewardPerToken 스냅샷
    mapping(address => uint256) public userRewardPaid;
    /// @dev checkpoint 시점까지 확정된 보상 — unstake 후에도 소실되지 않는다
    mapping(address => uint256) public userOwed;

    error ZeroAmount();
    error ZeroDuration();
    error InvalidToken();
    error InsufficientStake(uint256 wanted, uint256 staked);

    event RewardsFunded(uint256 amount, uint256 duration, uint256 rate, uint256 finishAt);
    event Staked(address indexed user, uint256 amount);
    event Unstaked(address indexed user, uint256 amount);
    event Claimed(address indexed user, uint256 amount);

    constructor(address guardian, IERC20 stakingToken_, IERC20 rewardToken_) SimpleBrake(guardian) {
        // F-03: 무코드 주소 거부 — 배포 단계에서 잡는다.
        if (address(stakingToken_).code.length == 0 || address(rewardToken_).code.length == 0) {
            revert InvalidToken();
        }
        stakingToken = stakingToken_;
        rewardToken = rewardToken_;
    }

    // ---------------------------------------------------------------- 보상 풀

    /// @notice 보상 토큰을 풀에 추가하고 duration 동안 선형 방출한다.
    ///         이미 채워진 풀의 미방출분과 합쳐서 새 rate을 계산한다.
    /// @dev 누구나 호출 가능 (후원 모델). 신규 예치 = 진입이라 brake에 걸린다.
    function fundRewards(uint256 amount, uint256 durationSec) external nonReentrant whenEntryOpen {
        if (amount == 0) revert ZeroAmount();
        if (durationSec == 0) revert ZeroDuration();

        _updateGlobal(); // fund 시점까지의 귀속을 먼저 마감한다
        SafeToken.pullExact(rewardToken, msg.sender, amount);

        // 잔액 기반 remaining: 스테이커 부재 구간의 방출 예정분도 회수된다
        uint256 remaining = rewardToken.balanceOf(address(this)) - totalDebt;
        rewardRate = remaining / durationSec;
        finishAt = block.timestamp + durationSec;
        updatedAt = block.timestamp;
        emit RewardsFunded(amount, durationSec, rewardRate, finishAt);
    }

    // ---------------------------------------------------------------- 스테이킹

    /// @notice 스테이크 토큰을 예치한다 (진입 — brake 차단).
    function stake(uint256 amount) external nonReentrant whenEntryOpen {
        if (amount == 0) revert ZeroAmount();
        _updateGlobal();
        _checkpoint(msg.sender);
        // F-02: 도착량 기준 — FoT 스테이크 토큰도 실제 도착만 크레딧
        uint256 delivered = SafeToken.pull(stakingToken, msg.sender, amount);
        totalStaked += delivered;
        userStaked[msg.sender] += delivered;
        emit Staked(msg.sender, delivered);
    }

    /// @notice 스테이크를 회수한다 (탈출 — brake와 무관하게 항상 열려 있다).
    ///         checkpoint가 진행분을 userOwed로 이월하므로 전액 인출 후에도
    ///         귀속 보상은 유지된다 (claim으로 수령).
    function unstake(uint256 amount) external nonReentrant {
        if (amount > userStaked[msg.sender]) revert InsufficientStake(amount, userStaked[msg.sender]);
        _updateGlobal();
        _checkpoint(msg.sender);
        userStaked[msg.sender] -= amount;
        totalStaked -= amount;
        SafeToken.push(stakingToken, msg.sender, amount);
        emit Unstaked(msg.sender, amount);
    }

    /// @notice 귀속된 보상을 수령한다 (탈출 — brake와 무관).
    /// @dev CEI: owed 소거와 totalDebt 감소를 먼저, 지급(push)을 마지막에.
    function claim() external nonReentrant returns (uint256 paid) {
        _updateGlobal();
        _checkpoint(msg.sender);
        paid = userOwed[msg.sender];
        if (paid > 0) {
            userOwed[msg.sender] = 0;
            totalDebt -= paid;
            SafeToken.push(rewardToken, msg.sender, paid);
        }
        emit Claimed(msg.sender, paid);
    }

    // ---------------------------------------------------------------- 뷰 (F-04: 상수 시간)

    /// @notice 현재까지 귀속된 사용자 보상 (이월분 + 진행분).
    function earned(address user) external view returns (uint256) {
        uint256 rpt = _rewardPerToken();
        return userOwed[user] + ((userStaked[user] * (rpt - userRewardPaid[user])) / _PRECISION);
    }

    /// @notice 현재 시점까지의 누적 rewardPerToken.
    function rewardPerToken() external view returns (uint256) {
        return _rewardPerToken();
    }

    // ---------------------------------------------------------------- 내부

    /// @dev 전역 누적치를 현재 시각까지 반영한다. totalStaked == 0인 구간은
    ///      분모가 없어 반영하지 않는다 — 그 예정분은 잔액에 남아 다음
    ///      fund의 remaining에 편입된다 (소실 없음).
    function _updateGlobal() private {
        uint256 nowTs = block.timestamp;
        uint256 t = nowTs < finishAt ? nowTs : finishAt;
        if (t > updatedAt && totalStaked != 0 && rewardRate != 0) {
            uint256 accrued = (t - updatedAt) * rewardRate;
            rewardPerTokenStored += (accrued * _PRECISION) / totalStaked;
            totalDebt += accrued;
        }
        updatedAt = nowTs;
    }

    function _rewardPerToken() private view returns (uint256) {
        if (totalStaked == 0 || rewardRate == 0) return rewardPerTokenStored;
        uint256 t = block.timestamp < finishAt ? block.timestamp : finishAt;
        if (t <= updatedAt) return rewardPerTokenStored;
        return rewardPerTokenStored + ((t - updatedAt) * rewardRate * _PRECISION) / totalStaked;
    }

    /// @dev 사용자 진행분을 userOwed로 확정하고 스냅샷을 당긴다.
    ///      _updateGlobal(전역 rpt 갱신) 직후에만 호출한다.
    function _checkpoint(address user) private {
        userOwed[user] += (userStaked[user] * (rewardPerTokenStored - userRewardPaid[user])) / _PRECISION;
        userRewardPaid[user] = rewardPerTokenStored;
    }
}
