# 05. 상태 계층

## 키 (EIP-7864 통합 32바이트)

```
stem(31바이트) = H(0^12 ‖ address(20) ‖ overflow(1) ‖ tree_index 하위 248비트 BE(31))[0..31]
key = stem ‖ sub_index(1바이트)
```
- go-ethereum bintrie V3 방식. `ubt` 크레이트와 키·루트가 일치함을 `crates/state/tests/eip7864_compat.rs`가 검증한다(SHA-256/BLAKE3 평문 해시로 교차 확인).
- 영(0) 규칙: 트리 노드 해시에서 전부 0인 입력은 ZERO. 값 0 쓰기는 삭제와 같다(EVM 미설정 = 0). 키 파생에는 영 규칙을 적용하지 않는다(레퍼런스와 동일).
- 계정 헤더 stem(`tree_index=0`): sub 0 = 기본 데이터(version | code_size u24 | nonce u64 | balance u128), sub 1 = 코드 해시, sub 64..127 = 스토리지 슬롯 0..63, sub 128..255 = 코드 청크 0..127
- 그 밖의 스토리지: 위치 = 256^31 + slot → `tree_index = 256^30 + slot >> 8`, `sub = slot % 256` (최상위 바이트가 0xff면 overflow 플래그)
- 코드 청크 128 이후: 위치 = 128 + chunk_id → `tree_index = pos / 256`, `sub = pos % 256`
- `H`는 Hasher trait (Poseidon2 또는 BLAKE3).

## 트리

- stem마다 값 256개의 8단 바이너리 서브트리(리프 = H(value), 빈 리프 = ZERO) → stem 노드 = H(stem ‖ 0x00 ‖ 서브트리 루트).
- stem들 위로는 stem 비트(248비트)에 대한 바이너리 트리. stem 하나만 있는 서브트리는 그 stem 노드로 축약, 빈 서브트리는 ZERO, 내부 노드는 compress(L, R) (둘 다 ZERO면 ZERO).
- 리프는 항상 해시된 값으로만 압축 함수에 들어간다(선해시, CCS 2026 요건).
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
