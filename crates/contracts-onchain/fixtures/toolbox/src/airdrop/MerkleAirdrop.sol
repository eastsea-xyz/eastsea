// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {MerkleProof} from "openzeppelin/utils/cryptography/MerkleProof.sol";
import {ReentrancyGuard} from "openzeppelin/utils/ReentrancyGuard.sol";

/// @title 예제 13 — 머클 에어드랍
/// @notice 32바이트 루트 하나로 임의 인원에게 자격을 증명해 청구한다.
///         청구는 마감까지, 마감 후 잔여는 distributor가 회수한다.
/// @dev
///  설계 결정 — 명단을 상태에 두지 않는다:
///   - 수령인 1만 명 명단을 매핑에 넣으면 1만 슬롯(100만 units)을
///     배포자가 선불한다. 머클 루트는 그 명단 전체를 32바이트로
///     커밋한다 — 체인은 약속만 보관하고, 명단은 오프체인에 산다.
///   - 자격 증명은 청구 시점의 32바이트 증명이 한다. 체인 상태는
///     (루트, 청구자당 1슬롯)이 전부다.
///   - 청구 슬롯은 최소다: bool 하나. 금액을 저장하지 않는다 —
///     금액은 증명의 입력이고 이벤트에만 나온다. 이 슬롯은
///     "이미 청구했는가"라는 1비트 질문에 1비트면 충분하기 때문.
///
///  남은 미청구액은 온체인으로 알 수 없다 (F-04):
///   - "얼마나 남았나"는 미청구 명단의 합 — 체인은 명단을 모른다.
///   - `balance - totalClaimed`는 **상한**이다 (후원 receive가
///     섞일 수 있으므로). 정확한 집계는 오프체인 인덱스의 몫이다.
///
///  마감과 회수:
///   - 청구 기간을 무한으로 두면 못 받은 잔여가 영구 잠긴다 —
///     에어드랍 풀은 반드시 회수 경로를 가져야 한다.
///   - 마감 후 sweep은 permissionless(누구나 트리거)하지만 수령인은
///     distributor로 고정 — 예제 9·10·12의 인도 패턴과 같다.
///   - distributor는 수령인일 뿐 키가 아니다: 루트·마감·청구에
///     간섭할 수 없다 (F-06).
///
///  brake가 없는 이유:
///   - SimpleBrake는 신규 진입(자금·권리 유입)을 막는다. 이
///     컨트랙트의 진입은 배포 시점(루트·마감 immutable)에 끝났다.
///     claim은 탈출(자격의 현금화), sweep은 탈출(잔여 회수) —
///     막을 진입이 없으므로 brake도 없다.
///
///  F 매핑:
///   F-01: claim/sweep CEI(claimed 플래그·totalClaimed 먼저, native
///         전송 마지막) + nonReentrant.
///   F-02/F-03: native만 — FoT·무코드 영역 밖. receive()는 풀
///         충전을 위해 허용(후원 가능 — totalClaimed와 무관).
///   F-04: 남은 미청구액 조회 없음 — balance가 상한, 집계는 오프체인.
///   F-05: 7702 위임 EOA 청구도 동일 — 권한은 리프의 주소 성분.
///   F-06: distributor는 sweep 수령만. 루트·마감·청구에 키 없음.
///   F-07: Claimed 이벤트 ≠ 청구 — 진실은 claimed 매핑이다.
///   F-08: 무작위성 없음 — 배정은 오프체인 명단(루트)이 결정했다.
contract MerkleAirdrop is ReentrancyGuard {
    /// @dev 분배 명단의 머클 루트 — 리프 = keccak256(abi.encodePacked(account, amount))
    bytes32 public immutable merkleRoot;

    /// @dev 청구 마감 — 이후 claim은 닫히고 sweep이 연다
    uint48 public immutable deadline;

    /// @dev 마감 후 잔여 회수 수령인 (배포 시 고정)
    address public immutable distributor;

    /// @dev 청구 완료 비트 — 청구자당 1슬롯, 청구가 만든다
    mapping(address => bool) public claimed;

    /// @dev 지급 누적 — 계량·이벤트 조회 대용 스칼라 (1슬롯, 최초 청구 시)
    uint256 public totalClaimed;

    error ZeroRoot();
    error ZeroDistributor();
    error DeadlineTooShort();
    error AlreadyClaimed();
    error ClaimClosed(uint256 at, uint256 until);
    error InvalidProof();
    error InsufficientPool(uint256 needed, uint256 have);
    error ClaimTransferFailed();
    error SweepTooEarly(uint256 at, uint256 until);
    error SweepTransferFailed();

    event Claimed(address indexed user, uint256 amount);
    event Swept(address indexed distributor, uint256 amount);

    /// @param merkleRoot_ 분배 명단 머클 루트
    /// @param distributor_ 마감 후 잔여 수령인
    /// @param claimPeriod  청구 기간 (배포 시각부터, 초)
    constructor(bytes32 merkleRoot_, address distributor_, uint48 claimPeriod) {
        if (merkleRoot_ == bytes32(0)) revert ZeroRoot();
        if (distributor_ == address(0)) revert ZeroDistributor();
        if (claimPeriod == 0) revert DeadlineTooShort();

        merkleRoot = merkleRoot_;
        distributor = distributor_;
        deadline = uint48(block.timestamp) + claimPeriod;
    }

    /// @dev 풀 충전 — 누구나 후원할 수 있다 (totalClaimed와 무관)
    receive() external payable {}

    // ---------------------------------------------------------------- 청구

    /// @notice 증명으로 자격을 보여주고 amount를 청구한다.
    /// @param amount 리프에 커밋된 금액
    /// @param proof  루트까지의 머클 증명
    function claim(uint256 amount, bytes32[] calldata proof) external nonReentrant {
        if (claimed[msg.sender]) revert AlreadyClaimed();
        if (block.timestamp > deadline) revert ClaimClosed(block.timestamp, deadline);

        bytes32 leaf = keccak256(abi.encodePacked(msg.sender, amount));
        if (!MerkleProof.verifyCalldata(proof, merkleRoot, leaf)) revert InvalidProof();
        if (address(this).balance < amount) revert InsufficientPool(amount, address(this).balance);

        claimed[msg.sender] = true; // CEI: 비트 먼저
        totalClaimed += amount;
        emit Claimed(msg.sender, amount);

        (bool ok,) = msg.sender.call{value: amount}("");
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
