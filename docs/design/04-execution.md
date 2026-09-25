# 04. 실행 계층

## 파이프라인 (순서 먼저)

```
높이 H 합의 ──▶ txs, BAL, inclusion_list 확정 (인증서)
                 │
                 ├─▶ 실행기: exec(H-LAG) … BAL로 정적 DAG → Block-STM → post_root(H-LAG)
                 │      └─▶ 헤더 H+1 에 state_root_at = (H-LAG, post_root)
                 │
                 └─▶ 증명기 풀: chunk(H-LAG) 분배 → ChunkProof → aggregate → BlockProof(H-LAG)
                        └─▶ 지갑: BlockProof 검증 → "H-LAG 까지 수학적으로 확인됨"
```

- LAG=1이면 실행 예산은 블록 주기 1개. Monad는 10블록 지연으로 400배 예산을 얻지만 우리는 증명이 별도로 뒤따르므로 1~2로 충분.
- 실행 실패(BAL 불일치, gas 초과)는 **블록 무효**. 합의 검증 단계에서 잡아야 하므로 검증자는 `validate()`에서 실행을 수행한다. 즉 검증자는 실행을 두 번 하지 않는다: 검증 시 1회, 결과를 캐시.

## BAL 생성·검증

생산자:
1. 멤풀에서 후보 tx 선택 (OrderingPolicy).
2. **동적 Block-STM으로 실행**하며 접근 집합 수집 → BAL.
3. 블록에 BAL 포함.

검증자:
1. BAL로 **정적 DAG** 구성: tx i → tx j 간선은 i가 쓰고 j가 읽는 키가 있을 때.
2. DAG 위상 순서로 병렬 실행 (grevm 2.1 방식). 충돌 검증 불필요.
3. 실행 중 BAL에 없는 키 접근 → 즉시 블록 무효.
4. 실행 후 접근 집합 == BAL 확인.

## Block-STM 재구현 (`execution/blockstm/`)

기존 구현 폐기. 구조:

```rust
pub struct Scheduler { execution_idx: AtomicUsize, validation_idx: AtomicUsize, ... }  // Aptos 논문 §3
pub struct MvMemory<K, V> { /* 키별 (tx_idx → Option<V> | ESTIMATE) */ }
pub enum Mode { StaticDag(Dag), Dynamic }
```

- `StaticDag` 모드에서는 `validation_idx`가 필요 없다(간선이 곧 의존성). `Dynamic`은 생산자 전용 및 BAL 없는 테스트.
- 정확성 근거: `tests/differential/` (05 참조). 결정성: 같은 입력 1,000회 실행 root 동일.
- 산술: 모든 잔액·가스 계산 `checked_*`. 오버플로는 revert.
- 인위적 부하 없음. 기존 SHA256 60회 루프 제거.

## revm 통합

- `revm 43.0.1`, `alloy-evm`으로 블록 실행 glue.
- Inspector 두 개: `ProveGasInspector`(opcode·프리컴파일별 사이클 계량), `AccessListInspector`(BAL 수집).
- 프리컴파일 화이트리스트: `0x01 ecrecover`, `0x02 sha256`, `0x05 modexp(크기 상한)`, `0x06-0x08 bn254`, `0x09 blake2f`, `0x100 P256VERIFY`. keccak은 opcode. 나머지는 비활성.
- 해시 opcode 가스는 EIP-7667 배수 적용. 상태 가스는 EIP-8037.

## prove-gas

- 테이블: `execution/prove_gas/table.rs`. 초기값은 zkVM 게스트에서 opcode별 사이클을 실측한 값(스파이크 S5). 프리컴파일은 ECALL 사이클.
- 블록 상한 `PROVE_GAS_LIMIT`은 "맥 1대 × 위원회 증명기 수 × 블록 주기" 로부터 역산.
- tx는 `header.gas.prove`를 선언하고, 초과 시 revert + 수수료 소각.

## 게스트 프로그램 (`guest/block-exec/`)

입력: `ChunkInput { pre_root, txs[range], bal_slice, witness }`.
동작: witness로 부분 상태 트리 복원 → revm으로 tx 순차 실행(게스트 안에서는 병렬 불필요) → post_root 계산 → `ChunkIo` 출력.
출력 커밋: `hash(pre_root ‖ post_root ‖ tx_range ‖ gas)`.

- 게스트는 `no_std` revm 경로. 프리컴파일은 zkVM ECALL로 매핑.
- 청크 경계는 BAL DAG의 **약한 연결 성분** 경계와 정렬해 witness 중복을 줄인다.

## 벤치마크 축

계정 수 {2, 10, 100, 1k, 10k} × 스레드 {1, 2, 4, 8, 10} × 워크로드 {송금, 30% swap, 배포+호출}.
기준: grevm 2.1 동일 워크로드 대비 ±20%. 절대 TPS는 공개하되 비교 기준으로 쓰지 않는다.


## 병렬 실행 — 구현됨 (2026-09-26, `crates/execution/src/parallel.rs`)

현재 BAL은 블록 단위 접근 집합(슬롯별 마지막 기록자, 계정 플래그)이라 tx별 의존 그래프를 만들 수 없다. 그래서 **낙관적 병렬 실행**(1라운드 Block-STM)을 쓴다. BAL은 계속 정확 일치로 검증한다.

1. 모든 tx의 서명 검사·페이로드 디코드를 병렬로 한 번만 한다(상태 무관, tx 비용의 대부분).
2. 모든 tx를 블록 사전 상태에 대해 병렬 투기 실행한다.
3. 블록 순서대로 커밋한다. 투기 결과가 읽은 계정(잔액·nonce·코드·존재)이나 슬롯을 앞선 tx가 바꿨으면 현재 상태에서 재실행한다.
4. 수수료 수취인(제안자)은 모든 tx가 쓰므로 수수료를 **현재 잔액 + 수수료**로 재계산해 적용한다. tx가 수취인을 다른 방식으로 봤으면(BALANCE·EXTCODE*·CALL 대상·SELFDESTRUCT·발신자/수신자) 인스펙터가 감지해 재실행한다.

결과는 순차 실행과 **바이트 단위로 같다**: proptest 차등 테스트(무작위 블록 48개: 송금·같은 카운터 경합·수취인 잔액 읽기·수취인 발신·잘못된 nonce)로 상태 루트·BAL·영수증·가스·포함 tx가 일치함을 확인. 대조 실험으로 충돌 검사를 끄거나 수취인 감지를 끄면 테스트가 실패함을 확인했다.

측정(M1 Max, release, P코어 8): 2,400 송금(24 발신자) 2.08배, 한 카운터에 600회 호출(전부 충돌) 1.84배, 발신자 240명 240 tx 2.23배. 남은 직렬 구간은 트리 커밋(Poseidon2 해시)이며 다음 병렬화 대상이다.
