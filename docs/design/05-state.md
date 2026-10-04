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
- 주소별 활동 색인(`account_history`)과 블록별 색인 키 목록(`account_block_keys`)도 같은 확정 블록 redb 트랜잭션에 기록한다. 디스크 오류나 재시작 때 내역이 체크포인트보다 앞서지 않는다. 블록 가지치기 때 키 목록으로 해당 주소 행만 제거한다. 활동 내역은 합의 상태가 아니며, 잔액 증명의 대안이 아니다.
- 한계: 상태 전체를 메모리에 올린다(맥 RAM 한도가 상태 크기 한도). 디스크 기반 트리 노드 페이징(스템 서브트리 해시 캐시를 디스크에 두고 필요 시 로드)이 다음 단계이며, 그때도 해시 규칙은 EIP-7864를 유지한다.

## 스냅샷

- `snapshot(height)`: NOMT 페이지 파일을 CoW 복사 → tar.zst → 2GiB 청크 → 매니페스트.

현재 노드의 인증된 체크포인트 스냅샷은 별도 postcard/AUN2 형식이다(`crates/node/src/snapshot.rs`, `rpc.rs`, `follow.rs`). RPC는 체인 뮤텍스 안에서 확정 상태의 `Arc`와 작은 메타데이터만 잡고, 전체 엔트리 복사·직렬화는 잠금 밖의 블로킹 작업자에서 한다. AUN2 머리말은 직렬화 버퍼 안에서 붙인다. 캐시 높이 120블록 동안 한 번만 만들며, 동시 재빌드 요청은 즉시 오류를 돌려준다. 실패한 재빌드는 30초 동안 제한한다. 만들기 전에 메모리 압력 정책을 적용한다: 압력이 CRITICAL이면 항상 거부하고(커널이 이미 생존 회수 중이므로), WARN(일상 사용 중인 소비자 Mac이 하루 대부분 머무는 수준)이나 NORMAL이면 엔트리·코드 크기의 보수적 추가 메모리 추정치를 현재 사용 가능 메모리의 1/4 및 노드의 `--max-memory` 상한과 비교해 초과할 때만 거부한다. 즉 WARN은 그 자체로 거부 사유가 아니라 예산 검사를 통과하면 빌드가 돌아가, 바쁘지만 건강한 Mac이 새 노드·복구 노드에 체크포인트 동기화를 계속 제공한다. 팔로워의 인바운드 예산 게이트(audit 3 A3-5)도 같은 함수(`resources::snapshot_memory_budget`)와 같은 정책을 쓴다. 청크 요청은 재빌드를 시작하지 않는다. 다운로드는 최대 8개 청크만 동시에 잡아 한 버퍼에 순서대로 더한다. **1GiB는 전송 상한이지 전체 메모리 상한이 아니다.** 인증서·상태 루트·코드 검증과 직렬화 형식은 그대로다. 새 제네시스 합성 엔트리 100,000개(전송 6,400,348바이트)의 `Snapshot::build`+직렬화 측정에서 추가 physical footprint의 1ms 표본 최고값은 8,568,896바이트, 사전 추정은 42,377,216바이트였다. 표본값은 실제 최대치의 하한이며 8GiB Mac 측정이 아니다. 클라이언트에서 조립 버퍼와 디코딩 엔트리는 디코딩 중 겹친다. 버퍼는 `check` 전에 해제되지만, `check`는 디코딩 엔트리를 복사해 상태를 재구축하므로 그 둘이 겹친다. 최소 사양 8GiB Mac에서 두 단계의 피크를 실측하고, 필요하면 디스크 스트리밍과 소비형 검증으로 바꿔야 한다. 이 코드는 합의나 테스트넷 7780의 블록 와이어 형식을 바꾸지 않는다.
- 최신 스냅샷 2개만 R2/B2에 유지, 마일스톤은 Zenodo.
- 지갑은 스냅샷을 받지 않는다. root + 자기 계정 multiproof만 받는다.

## 이력 만료

- 검증자: 최근 `HISTORY_KEEP` 블록(기본 30일)만 로컬. 그 이전은 에라 루트만.
- 에라(8,192블록, 이력 v2) 해시는 다음 블록 헤더가 아니라 **별도 이력 커밋 트리**의 root로 헤더에 포함(`history_root`, 2장 헤더에 있음 — 7780 제네시스부터).
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
