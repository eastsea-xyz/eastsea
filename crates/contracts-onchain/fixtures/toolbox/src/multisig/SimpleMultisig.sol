// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title 예제 8 — 단순 멀티시그 (K-of-N)
/// @notice 서명 임계치를 채우면 누구나 트랜잭션을 실행할 수 있는
///         정적 소유자 멀티시그. 서명은 오프체인에서 수집하고
///         실행 트랜잭션 하나로 제출한다 (relayer 패턴).
/// @dev
///  설계 결정 — 유료 상태 최소화:
///   - 컨펌을 온체인에 쌓지 않는다. 온체인 컨펌 모델(Gnosis 계열)은
///     tx마다 컨펌 비트맵을 기록하지만, 여기는 서명을 오프체인에
///     두고 execute 한 번으로 끝낸다. 체인에 남는 것은
///     executed[hash] 슬롯 1개뿐이다.
///   - 서명은 오름차순·고유 정렬을 강제한다 (recovered > last).
///     정렬 검사가 중복 서명 카운팅을 자동으로 막는다.
///   - 소유자 목록은 정적이다. 교체는 멀티시그가 새 multisig를
///     배포해 잔액을 옮기는 방식 — 변경 로직 자체가 공격면이 된다.
///
///  재생 방지 — 해시에 도메인(nonce, chainId, 자기 주소)이 들어간다:
///   같은 tx라도 nonce를 바꾸면 다른 해시, 다른 체인·다른 multisig
///   인스턴스에서는 어차피 다른 해시가 나온다.
///
///  F 매핑:
///   F-01: executed[hash]를 call보다 먼저 true로 만든다(CEI) —
///         execute가 재귀적으로 자신을 호출해도 두 번째는
///         AlreadyExecuted로 죽는다. 가드 불필요.
///   F-02: 토큰 전송은 data 필드의 임의 호출로 처리 — 멀티시그 자체는
///         토큰 회계를 갖지 않아 좌초 문제가 없다.
///   F-03: 대상 call은 EOA 송금(value>0, data 빈)도 정상 처리한다.
///   F-04: getTransactionHash 상수 시간.
///   F-05: 7702로 위임된 EOA 대상 호출도 일반 call과 동일하게 동작.
contract SimpleMultisig {
    /// @dev 배포 시 고정 도메인 — 재생 방지의 핵심
    bytes32 public immutable domainSeparator;

    uint256 public immutable threshold;

    mapping(address => bool) public isOwner;

    /// @dev 실행 완료 트랜잭션 해시 — 실행당 1슬롯, 유일한 가변 상태
    mapping(bytes32 => bool) public executed;

    error ZeroThreshold();
    error OwnerCountMismatch(uint256 owners, uint256 threshold);
    error OwnersNotSorted();
    error ZeroAddress();
    error InvalidSignature();
    error AlreadyExecuted();
    error InsufficientConfirmations(uint256 got, uint256 want);
    error ExecutionFailed(bytes ret);

    event Executed(bytes32 indexed txHash, address indexed to, uint256 value, uint256 nonce, uint256 confirmations);

    constructor(address[] memory owners_, uint256 threshold_) {
        uint256 n = owners_.length;
        if (threshold_ == 0 || threshold_ > n) revert OwnerCountMismatch(n, threshold_);
        if (n == 0) revert ZeroThreshold();

        address last = address(0);
        for (uint256 i; i < n; ++i) {
            address o = owners_[i];
            if (o == address(0) || o <= last) revert OwnersNotSorted(); // 고유+정렬 강제
            isOwner[o] = true;
            last = o;
        }

        threshold = threshold_;
        // constructor 시점 address(this)는 확정 — 체인 간 재생 차단
        domainSeparator = keccak256(abi.encode(block.chainid, address(this)));
    }

    // ---------------------------------------------------------------- 실행

    /// @notice 서명 묶음이 임계치를 충족하면 임의 호출을 실행한다.
    ///         실행자는 누구든 (relayer) — 권한은 서명에 있다.
    /// @param to         호출 대상
    /// @param value      함께 보낼 native coin
    /// @param data       호출 데이터 (토큰 transfer, 배포, 무엇이든)
    /// @param nonce      서명 시점에 지정한 값 — 재실행 방지 식별자
    /// @param signatures 오름차순 정렬된 65바이트 서명 배열
    function execute(address to, uint256 value, bytes calldata data, uint256 nonce, bytes[] calldata signatures)
        external
        returns (bytes memory)
    {
        bytes32 h = getTransactionHash(to, value, data, nonce);
        if (executed[h]) revert AlreadyExecuted();
        executed[h] = true; // CEI: 재진입해도 두 번째는 죽는다

        uint256 confirmations = _checkSignatures(h, signatures);
        emit Executed(h, to, value, nonce, confirmations);

        (bool ok, bytes memory retData) = to.call{value: value}(data);
        if (!ok) revert ExecutionFailed(retData);
        return retData;
    }

    // ---------------------------------------------------------------- 뷰 (F-04)

    /// @notice 서명 대상 해시 — 지갑에서 personal_sign으로 서명한다.
    ///         도메인(chainId + 컨트랙트 주소)·nonce가 해제에 들어가
    ///         체인 간·인스턴스 간·재실행 재생을 한 번에 차단한다.
    function getTransactionHash(address to, uint256 value, bytes calldata data, uint256 nonce)
        public
        view
        returns (bytes32)
    {
        bytes32 inner = keccak256(abi.encode(domainSeparator, to, value, keccak256(data), nonce));
        return keccak256(abi.encodePacked("\x19Ethereum Signed Message:\n32", inner));
    }

    // ---------------------------------------------------------------- 내부

    /// @dev 서명 검증 — recovered 주소가 엄격 오름차순이어야 한다:
    ///      중복 서명(같은 주소 2회)과 address(0)(무효 서명)가
    ///      자동으로 거부된다. 비소유자 서명이 섞이면 전체가 죽는다
    ///      (스킵하지 않는다 — 실수를 조용히 넘기지 않는다).
    function _checkSignatures(bytes32 h, bytes[] calldata signatures) private view returns (uint256 count) {
        address last = address(0);
        uint256 n = signatures.length;
        if (n < threshold) revert InsufficientConfirmations(n, threshold);

        for (uint256 i; i < n; ++i) {
            bytes calldata sig = signatures[i];
            if (sig.length != 65) revert InvalidSignature();

            bytes32 r;
            bytes32 s;
            uint8 v;
            // calldata에서 직접 절편 — 메모리 복사 없음
            assembly {
                r := calldataload(sig.offset)
                s := calldataload(add(sig.offset, 32))
                v := byte(0, calldataload(add(sig.offset, 64)))
            }

            address recovered = ecrecover(h, v, r, s);
            if (recovered <= last) revert InvalidSignature(); // 0(무효)·중복·미정렬
            if (!isOwner[recovered]) revert InvalidSignature();
            last = recovered;
            ++count;
        }
        if (count < threshold) revert InsufficientConfirmations(count, threshold);
    }
}
