# 06. 증명 계층

## 백엔드 (스파이크 S1·S2 후 확정)

| 후보 | 상태 | feature |
|---|---|---|
| Lattice Jolt (feat/akita-metal) | Metal 네이티브, 미감사, reth 게스트 없음 → 우리가 게스트 작성 | `jolt` |
| Stwo M31 + WHIR | Apple GPU 최고 ops/s, 프로덕션. 증명 200~600KB → 재귀 래핑 필요 | `stwo` |
| RISC Zero 3.x | Metal "deprecated" 의미 확인 필요 | `risc0` |

`proving/src/backend/{jolt,stwo,risc0}.rs` 각각 `Prover` 구현. 게스트는 RV64IM 공통, 백엔드별 툴체인.

## 청크 분담

```
BlockProof(H) 요청
  ├─ 청크 계획: BAL DAG 약한 연결 성분 → 청크 k개, 각 청크 예상 사이클 ≈ 균등 (Advisor 힌트 사용 가능)
  ├─ 분배: 위원회 증명기 목록 → 청크 할당 (가용 GPU, 최근 처리 속도 가중)
  ├─ 각 증명기: prove_chunk → ChunkProof → gossip
  ├─ 집계: 2-to-1 재귀 트리, 누구든 두 ChunkProof를 받으면 합칠 수 있음 (경쟁 허용, 중복 무해)
  └─ BlockProof → gossip → 지갑
```

- 청크 io 연결 조건: `chunk[i].post_root == chunk[i+1].pre_root`.
- 집계 회로는 `Pcs` trait 위에서 직접 작성(zkVM 안에서 Verifier 실행하는 재귀). leanVM 기준 2-to-1 약 200ms 목표.
- 증명기 0대일 때: 검증자가 자기 블록을 순차 증명(느리지만 진행).

## 증명 검증기 (지갑용)

- `proving/src/verifier/` 는 `no_std`, alloc만. 의존성은 필드 연산·해시뿐.
- 빌드 타깃: macOS, iOS, wasm32. UniFFI로 `verify_block(proof_bytes, expected_io) -> bool` 노출.
- 목표: 검증 ≤100ms (M1), 메모리 ≤64MB, 증명 ≤300KiB.

## Metal 최적화 (2단계 기여)

우선순위는 스파이크 S5 프로파일로 정한다. 후보:
1. Poseidon2/Merkle 커밋 커널 (ICICLE Metal에 없음)
2. sumcheck 라운드 (메모리 대역폭 bound → 통합 메모리 이점)
3. 세그먼트 분할 제거: 트레이스를 통합 메모리에 상주, CUDA의 VRAM 분할 오버헤드 회피
4. NTT/DCCT (M31 Circle)
5. AMX 필드 연산 실험 (선례 없음)

업스트림 대상: Jolt `jolt-kernels/metal`, Stwo(ICICLE-Stwo 또는 자체 Metal 백엔드).

## 증명 대기열과 Advisor

- `ProofQueue`: (높이, 청크) 우선순위 큐. 기본 정책 = 오래된 높이 우선.
- Advisor 힌트: 청크 예상 시간, 증명기 가용성 예측. 힌트가 틀려도 정확성 무관.
- "증명 전 조기 경보": 증명 미도착 블록에 이상 점수 → 지갑 UI "대기 중" 표시 강도만 바꿈.

## 실패 처리

- 증명 검증 실패: 해당 블록 격리, 위원회에 경보, 재증명 요청. BFT 인증서와 증명이 계속 불일치하면 체인 정지(수동 개입). 이 사건은 곧 "실행 버그 또는 위원회 담합"이라 자동 복구하지 않는다.
- 증명 지연 > `PROOF_TIMEOUT`(예: 10블록): 지갑은 "N블록 미확인" 경고, 송금 한도 축소 제안(사용자 설정).

## 측정 항목 (CI·CodSpeed)

cycles/s (CPU, Metal), 청크당 증명 시간, 집계 시간, 증명 크기, 검증 시간(M1·iPhone·wasm), 피크 메모리.
