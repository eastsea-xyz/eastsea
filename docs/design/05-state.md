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

## 저장 엔진 — 변경: NOMT 대신 EIP-7864 트리 + redb 영속화 (2026-09-26)

**NOMT를 엔진으로 쓰지 않는다.** nomt-core 1.0.5의 `NodeHasher`는 32바이트 해시만 보고 노드 종류를 판별해야 해서(`node_kind`), 리프·내부 노드 해시의 최상위 비트를 태깅한다(`set_msb`/`unset_msb`). 그 결과 루트가 EIP-7864(geth V3/ubt와 교차 검증한 값)와 같아질 수 없다. 지갑 경량 검증과 이더리움 호환 증명이 이 루트에 걸려 있으므로 트리 해시 규칙을 바꾸지 않는다.

지금 구현 (`crates/node/src/store.rs`):
- 트리 계산은 기존 `BinaryTree`(EIP-7864, Poseidon2) 그대로. `WorldState`가 블록마다 쓰기 저널(트리 쓰기 + 새 바이트코드)을 남긴다.
- 확정 블록마다 redb 트랜잭션 하나로 `state`(32B 키→32B 값, 삭제 반영) · `code` · `blocks`(요약) · `receipts` · `meta(height, digest, root)`를 커밋. **디스크 먼저, 메모리 헤드는 그다음**이라 크래시 후 디스크가 메모리보다 뒤처지지 않는다.
- 재시작: 저장된 엔트리로 트리를 다시 만들고 루트가 체크포인트 루트와 같아야 기동(다르면 `RootMismatch`로 중단). 체크포인트 이후 블록만 아카이브에서 재실행.
- 한계: 상태 전체를 메모리에 올린다(맥 RAM 한도가 상태 크기 한도). 디스크 기반 트리 노드 페이징(스템 서브트리 해시 캐시를 디스크에 두고 필요 시 로드)이 다음 단계이며, 그때도 해시 규칙은 EIP-7864를 유지한다.

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
