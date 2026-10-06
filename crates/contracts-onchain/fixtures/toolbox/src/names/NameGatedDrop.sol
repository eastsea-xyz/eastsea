// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {EastSeaNames} from "src/system/EastSeaNames.sol";
import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";

/// @title 예제 15 — 이름 자격 드롭 (이름 서비스 연동)
/// @notice primary name(.aeth)을 가진 계정만 받을 수 있는 고정액
///         드롭. 자격 검증을 시스템 이름 서비스에 통째로 위임한다.
/// @dev
///  설계 결정 — 신원은 빌려 쓰고 상태는 만들지 않는다:
///   - 자격 증명을 자체 매핑으로 만들면(화이트리스트) 배포자가
///     명단 상태를 선불한다 (예제 13이 머클로 피한 비용).
///   - 여기서는 `names.reverseOf(msg.sender) != ""`이 자격의 전부다.
///     시스템 이름 서비스의 reverseOf는 **정직성 조건**까지 검사해
///     준다 — forward 레코드가 살아 있고 그 주소를 되돌아가는
///     경우만 이름을 반환한다. 만료·스웜·위조는 모두 ""다.
///   - 앱은 이름 문자열을 상태에 저장하지 않는다. 자격 판정에
///     필요한 건 "있는가"뿐이고, 청구 기록은 주소 비트 1개다.
///     이름은 Claimed 이벤트의 페이로드로만 남는다 — 문자열은
///     영구 상태(1B=1u)가 아니라 로그(스캔 가능, 회수 불가능)에.
///
///  조회 시점 원칙:
///   - 자격은 claim 트랜잭션이 실행되는 순간의 이름 서비스 상태로
///     판정된다. 등록 시점에 해석해 두면(캐시) 이름이 만료·이전된
///     뒤에도 자격이 남는다 — 스푸핑 표면이다.
///   - 이름을 이전받은 새 소유자는 자기 primary name이 그 이름이라면
///     청구할 수 있다. 자격의 주체는 어디까지나 "지금 그 이름을
///     대표하는 계정"이다.
///
///  F 매핑:
///   F-01: claim/sweep CEI + nonReentrant — claimed 비트를 전송보다 먼저.
///   F-02/F-03: native만. receive() 허용 — 풀 충전.
///   F-04: 자격 판정은 시스템 뷰 호출 1회 (reverseOf) — 순회 없음.
///   F-05: 7702 위임 EOA도 동일 — primary name은 계정 주소 기준.
///   F-06: names·distributor·deadline·dropAmount 전부 immutable.
///         distributor는 마감 후 잔여 수령인일 뿐 키가 아니다.
///   F-07: Claimed 이벤트 ≠ 청구 — 진실은 claimed 매핑.
///   F-08: 무작위성 없음 — 자격 배정은 이름 서비스가 이미 결정.
///
///  brake가 없는 이유: 진입(자금·권리 유입)이 없다 — 자격은 외부
///  시스템의 상태이고 이 컨트랙트의 상수는 배포 시 고정됐다.
///  claim(탈출)과 sweep(탈출)만 남는다 (예제 13과 동일 논리).
contract NameGatedDrop is ReentrancyGuard {
    /// @dev 시스템 이름 서비스 — 자격의 원천
    EastSeaNames public immutable names;

    /// @dev 마감 후 잔여 회수 수령인 (배포 시 고정)
    address public immutable distributor;

    /// @dev 청구 마감
    uint48 public immutable deadline;

    /// @dev 1인당 지급액 (고정)
    uint256 public immutable dropAmount;

    /// @dev 청구 완료 비트 — 청구자당 1슬롯, 청구가 만든다
    mapping(address => bool) public claimed;

    /// @dev 지급 인원 — 스칼라 (예산 소진 조회용, 상수 시간)
    uint256 public claimCount;

    error ZeroAmount();
    error ZeroDistributor();
    error ZeroPeriod();
    error NoPrimaryName(address account);
    error AlreadyClaimed();
    error ClaimClosed(uint256 at, uint256 until);
    error InsufficientPool(uint256 needed, uint256 have);
    error ClaimTransferFailed();
    error SweepTooEarly(uint256 at, uint256 until);
    error SweepTransferFailed();

    event Claimed(address indexed user, string name);
    event Swept(address indexed distributor, uint256 amount);

    /// @param names_       시스템 이름 서비스 주소
    /// @param distributor_ 마감 후 잔여 수령인
    /// @param dropAmount_  1인당 지급액
    /// @param claimPeriod  청구 기간 (배포 시각부터)
    constructor(address names_, address distributor_, uint256 dropAmount_, uint48 claimPeriod) {
        if (dropAmount_ == 0) revert ZeroAmount();
        if (distributor_ == address(0)) revert ZeroDistributor();
        if (claimPeriod == 0) revert ZeroPeriod();

        names = EastSeaNames(names_);
        distributor = distributor_;
        dropAmount = dropAmount_;
        deadline = uint48(block.timestamp) + claimPeriod;
    }

    /// @dev 풀 충전 — 누구나 후원 가능 (claimCount와 무관)
    receive() external payable {}

    // ---------------------------------------------------------------- 청구 (탈출)

    /// @notice primary name을 가진 계정에 dropAmount를 지급한다.
    ///         자격·정직성 검증은 names.reverseOf가 대신한다.
    function claim() external nonReentrant {
        if (claimed[msg.sender]) revert AlreadyClaimed();
        if (block.timestamp > deadline) revert ClaimClosed(block.timestamp, deadline);

        string memory name = names.reverseOf(msg.sender); // 정직한 primary name만
        if (bytes(name).length == 0) revert NoPrimaryName(msg.sender);
        if (address(this).balance < dropAmount) revert InsufficientPool(dropAmount, address(this).balance);

        claimed[msg.sender] = true; // CEI: 비트 먼저
        ++claimCount;
        emit Claimed(msg.sender, name); // 문자열은 로그에만 — 상태 아님

        (bool ok,) = msg.sender.call{value: dropAmount}("");
        if (!ok) revert ClaimTransferFailed();
    }

    // ---------------------------------------------------------------- 회수 (탈출)

    /// @notice 마감 후 잔여 전액을 distributor에게 인도한다. 누구나 호출.
    function sweep() external nonReentrant {
        if (block.timestamp <= deadline) revert SweepTooEarly(block.timestamp, deadline);

        uint256 amount = address(this).balance;
        (bool ok,) = distributor.call{value: amount}("");
        if (!ok) revert SweepTransferFailed();
        emit Swept(distributor, amount);
    }
}
