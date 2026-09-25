# 05. 상태 계층

## 키 (EIP-7864 통합 32바이트)

```
stem(31바이트) = H(address ‖ tree_index)[0..31]
key = stem ‖ sub_index(1바이트)
```
- 계정 기본 데이터: `tree_index=0`, sub 0..3 = {version|balance|nonce|code_hash} 압축 ; sub 64.. = 코드 청크
- 스토리지: `tree_index = slot / 256`, `sub = slot % 256`
- `H`는 Hasher trait (Poseidon2 또는 BLAKE3). `ubt` 크레이트의 파생 규칙을 따른다.

## 트리

- 바이너리 sparse Merkle tree, 깊이 256 (키 비트).
- 리프 = `hash_leaf(key, value)` (항상 선해시, CCS 2026).
- 빈 서브트리 해시는 깊이별 사전 계산.
- multiproof: 키 집합에 대한 형제 노드 최소 집합. 형식은 NOMT의 witness를 그대로 직렬화.

## 저장 엔진: NOMT

- `NomtRepo`가 `StateRepository` 구현. NOMT의 hasher를 우리 `Hasher`로 주입.
- 커밋 단위 = 블록 실행 1회. 각 커밋은 `(height, root)`를 메타 테이블에 기록.
- macOS에서는 io_uring 대신 표준 I/O fallback (NOMT가 지원). 성능 기준선은 Linux CI에서 잰다.

## 스냅샷

- `snapshot(height)`: NOMT 페이지 파일을 CoW 복사 → tar.zst → 2GiB 청크 → 매니페스트.
- 최신 스냅샷 2개만 R2/B2에 유지, 마일스톤은 Zenodo.
- 지갑은 스냅샷을 받지 않는다. root + 자기 계정 multiproof만 받는다.

## 이력 만료

- 검증자: 최근 `HISTORY_KEEP` 블록(기본 30일)만 로컬. 그 이전은 청크 해시만.
- 청크(1,000블록) 해시는 다음 블록 헤더가 아니라 **별도 이력 커밋 트리**의 root로 헤더에 포함(`history_root`, 2장 헤더에 추가 예정).
- 아카이브 노드(poc-nas)만 전체 보관.

## 해시 전환 계획

- 제네시스: Poseidon2⟨KoalaBear, width 16⟩. 매개변수 NUMS 생성, 절차 공개.
- 트리거: 우리 증명기가 Flock 계열 배치 Boolean 가젯을 얻어 BLAKE3 증명 비용이 Poseidon2의 3배 이내로 떨어지면 BLAKE3로 하드포크. 마이그레이션은 EIP-7864 overlay 방식(구 트리 동결, 신 트리 누적).
- Poseidon 암호분석 이니셔티브 결과(2026-12)를 모니터링.

## 상태 증인(witness) 형식

```rust
pub struct StateWitness { pub root: Hash, pub keys: Vec<UnifiedKey>, pub values: Vec<Option<Bytes>>, pub siblings: Vec<Hash>, pub bitmap: BitVec }
```
게스트는 이걸로 부분 트리를 복원하고 실행 후 새 root를 계산한다. 크기는 BAL 키 수에 비례하므로 BAL이 곧 witness 예산.
