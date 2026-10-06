// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @notice 체인 프리컴파일 형태의 랜덤ness 뷰 (배포본: 0x…7704).
/// @dev 단일 제안자는 임계값 서명을 왜곡할 수 없지만, 그 word는 목표 에포크가
///      열리기 전에 공개된다(F-08). 커밋은 randomness(T) == 0일 때만,
///      리빌은 0이 아닌 뒤에만 받아야 한다.
interface IRandomness {
    /// @param epoch 에포크 번호 (블록 높이 아님)
    /// @return word 0 = 아직 열리지 않았거나 서명된 시드가 없음
    function randomness(uint64 epoch) external view returns (uint256 word);
}
