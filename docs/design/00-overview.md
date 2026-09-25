# Aether 상세 구현 설계서

작성일: 2026-09-26. 근거: `docs/research/` 12건 (모두 2026-09-25~26 확인).
개념 설계서(공유 문서)의 13개 장을 구현 단위로 구체화한 것이다.

## 문서 구성

| 파일 | 내용 |
|---|---|
| `00-overview.md` | 이 문서. 정체성, 원칙, 결정 목록, 미결 항목 |
| `01-workspace.md` | 크레이트 구조, 의존성, 버전 고정 |
| `02-types.md` | 트랜잭션 봉투, 블록, BAL, 인증서, 매니페스트 형식 |
| `03-traits.md` | 교체 가능 경계: PCS, 해시, 합의, DA, 서명자, 저장소 |
| `04-execution.md` | 순서-먼저 파이프라인, BAL 정적 DAG Block-STM, prove-gas |
| `05-state.md` | 바이너리 SMT, EIP-7864 키, NOMT, 스냅샷 |
| `06-proving.md` | zkVM 게스트, 청크 분담, 재귀 집계, 증명 검증기 |
| `07-consensus.md` | Simplex 통합, VRF 위원회, ebb-and-flow, Quint 명세 |
| `08-network.md` | iroh, Pkarr, Celestia DA 어댑터, 이력 배포 |
| `09-wallet.md` | UniFFI 경계, Secure Enclave 계정, 앱 3단 구조 |
| `10-testing.md` | 차등 테스트, 결정적 시뮬레이션, 벤치 게이트 |
| `11-phase0-spike.md` | 0단계 보안 정리와 0.5단계 스파이크 작업 목록 |

## 정체성

> 맥이 있으면 누구나 검증자다. 내 맥이 직접 검증하는 지갑.

이더리움이 2027~2029년에 도착할 설계(EIP-8025 증명으로 재실행 대체, EIP-7928 BAL,
EIP-7864 바이너리 트리, 해시 전용, P-256 계정, FOCIL)를 레거시 없이 지금 맥 위에서 구현한다.

## 설계 원칙

1. **증명이 중심.** 모든 계층의 선택 기준은 증명 비용이다.
2. **교체 가능 경계.** PCS, 해시, 합의, DA, 서명자, 저장소는 trait 뒤에 둔다. 어느 베팅이 이겨도 따라간다.
3. **검증 우선.** 차등 테스트, 결정적 시뮬레이션, Quint 명세를 1일차부터. 정확성 미통과 성능 수치는 기록하지 않는다.
4. **무임승차는 차별화하지 않는 층에만.** 직접 만드는 것은 Metal 증명 최적화, BAL Block-STM, 지갑 UX, 연결부.
5. **숫자로 통과.** 모든 게이트는 측정 가능하다.
6. **정직한 상한.** 처리량은 증명 상한으로 정한다. 초기 목표 수백 TPS.

## 확정된 결정 (스파이크 전)

| # | 결정 | 근거 |
|---|---|---|
| D1 | 실행: revm 43.0.1, RV64IM_Zicclsm 게스트 | nextgen-execution, nextgen-zk |
| D2 | BAL(EIP-7928)을 블록 유효성 조건으로 | nextgen-execution |
| D3 | 순서 먼저, 실행·증명 1~2블록 뒤 (Monad식) | nextgen-execution |
| D4 | prove-gas 3차원, 블록당 상한 | nextgen-execution |
| D5 | 상태: 바이너리 SMT, EIP-7864 32바이트 키. 저장은 redb(NOMT는 해시 MSB 태깅으로 EIP-7864 루트와 불일치, 05장) | nextgen-state |
| D6 | 해시 trait: Poseidon2⟨KoalaBear,16⟩ 기본, BLAKE3 선택 | nextgen-state |
| D7 | PCS trait: WHIR 오늘, Akita 감사 후 교체 | nextgen-zk |
| D8 | 합의: Commonware simplex + BLS 임계 scheme | nextgen-consensus |
| D9 | 검증자 전원 아님. VRF 고가동 위원회 + ebb-and-flow | nextgen-consensus |
| D10 | 계정: EIP-7702 위임 EOA + P-256 Secure Enclave, P256VERIFY | nextgen-wallet |
| D11 | DA: Celestia 소버린, 이더리움 blob은 trait 뒤 | nextgen-da-mev-ai |
| D12 | MEV: FOCIL 포함 목록(1단계) → tle(2단계) | nextgen-da-mev-ai |
| D13 | 네트워크: iroh 1.2, Pkarr, STUN | network-infra |
| D14 | 이력: 매니페스트 + HTTP/iroh-blobs 다중 미러 | ipfs-alternatives |
| D15 | 지갑: Rust 코어 → UniFFI → SwiftUI, Sparkle 배포 | nextgen-wallet |
| D16 | AI: 합의 밖. 증명 스케줄링·자원 관리·조기 경보만. tract CPU | nextgen-da-mev-ai |
| D17 | 증명 목표: 128비트 보안, ≤300KiB | nextgen-zk |
| D18 | 형식 명세: Quint → Rust 모델 기반 테스트 | nextgen-consensus |

## 스파이크 후 확정할 항목

| # | 항목 | 판단 기준 |
|---|---|---|
| S1 | zkVM: Lattice Jolt vs Stwo M31 | 맥 1대 cycles/s 실측, Jolt >1,000만 재현 여부 |
| S2 | RISC Zero Metal 상태 | deprecated가 "기본 활성"인지 "지원 종료"인지 |
| S3 | 노드 기반: reth SDK vs 경량(Commonware+grevm+NOMT) | 노트북 RSS·바이너리 크기 실측 |
| S4 | 블록 크기·주기 | 맥 1대 증명 시간을 블록 주기 N배 안에 |
| S5 | 병목 커널 우선순위 | NTT/해시/sumcheck 프로파일 비율 |
| S6 | 이력 P2P: iroh-blobs vs librqbit | 통합 난이도, webseed 필요성 |

## 미결 (사용자 답변 필요)

- 팀 규모와 "우리"의 실체
- 외부 감사 예산 (없으면 4단계를 "실험용, 감사 없음"으로 정의)
- Apple Organization 계정 (iOS App Store 배포 시 필수)

## 용어

| 용어 | 뜻 |
|---|---|
| BAL | Block-level Access List. 블록이 접근하는 모든 (주소, 슬롯) 집합. 유효성 조건 |
| prove-gas | zkVM 사이클 기반 3차원 가스 |
| 위원회 | VRF로 선출된 고가동 BFT 참여자 부분집합 |
| 검증 노드 | 증명만 확인하는 노드 (지갑 기본값) |
| 검증자 | 실행·투표하는 노드 (위원회 후보) |
| 증명기 | 청크를 증명하는 노드 |
