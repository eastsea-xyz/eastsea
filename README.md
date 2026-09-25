# Aether Node

> 맥이 있으면 누구나 검증자다. 내 맥이 직접 검증하는 지갑. **실제로 동작하는 네트워크**를 목표로 개발 중입니다.

[![Legal Disclaimer](https://img.shields.io/badge/Legal-Disclaimer%20%26%20Terms-red.svg)](DISCLAIMER.md)

> [!IMPORTANT]
> Aether Node is experimental, non-commercial research software provided **"AS IS"**. It is not a production blockchain, has not been audited, and all tokens (AETH) and rewards are simulated test artifacts with **zero monetary value**. See [DISCLAIMER.md](DISCLAIMER.md).

## 현재 상태

목표는 실동작입니다. 다만 지금 저장소의 코드는 아직 단일 머신에서만 도는 **초기 구현**이며, 아래 왼쪽 열은 1단계에서 전부 실제 구현으로 교체합니다.

| 영역 | 현재 코드 (교체 대상) | 1단계 실제 구현 |
|---|---|---|
| 합의 | 한 노드가 가상 검증자 5명의 vertex를 혼자 생성 (실제 합의 아님) | Commonware Simplex BFT + VRF 위원회 |
| 병렬 실행 | Block-STM 시도. **순차 실행과 결과가 일치하지 않음** (재구현 예정) | BAL 기반 정적 DAG Block-STM |
| 멤풀 암호화 | 고정 키 XOR. **MEV를 막지 못함** | 포함 목록 → 타임락 암호화 |
| 서명 | 없음 | EIP-7702 + Secure Enclave P-256 |
| 검증 | 재실행 | ZK 증명 검증 (맥 Metal 증명기) |
| 보상 | 라운드마다 로컬 계정에 +10 (가치 없음) | 없음 (토큰 설계는 법률 검토 전 보류) |

성능 수치는 [재현 가능한 벤치마크](docs/design/10-testing.md)가 정확성 검증을 통과한 뒤에만 공개합니다. 이전 README의 TPS·메모리 수치는 검증되지 않아 삭제했습니다.

## 설계 문서

- [상세 구현 설계서](docs/design/00-overview.md) — 정체성, 확정 결정 D1~D18, 계층별 설계
- [조사 보고서](docs/research/) — 2026-09 기준 근거 12건
- [0단계·스파이크 계획](docs/design/11-phase0-spike.md)

## 실행 (개발자)

```bash
git clone https://github.com/kjaylee/aether-node.git
cd aether-node
cargo run --release --bin aether-node            # 127.0.0.1:8080, 로컬 전용
cargo run --release --bin aether-node -- --public  # LAN/UPnP/DHT 공개 (주의)
```

- 기본은 **로컬 전용**(127.0.0.1)입니다. 외부 공개는 `--public`을 명시해야 하며, 이 경우 공유기 포트 개방(UPnP)과 공개 DHT 광고가 켜집니다.
- 대시보드는 브라우저로 자동으로 열립니다(`--no-open`으로 끔). 제어 API는 실행마다 생성되는 토큰(`~/.aether/token`, 권한 0600)이 필요합니다.
- 원격 피어가 보낸 gossip·sync로는 이 노드의 상태가 바뀌지 않습니다(서명 도입 전까지).

## 테스트

```bash
cargo test --workspace                          # 전체
cargo test -p aether-core --test security       # legacy 노드 보안 테스트
cargo test -p aether-state --test eip7864_compat  # EIP-7864 레퍼런스 교차 검증
```

## 프로젝트 구조

```
crates/                 # 차세대 구현 (docs/design 기준)
├── types/              # 봉투(3차원 가스), 블록(순서-먼저 헤더), BAL, 인증서, 증명, 매니페스트
├── hash/               # Hasher trait: Poseidon2<KoalaBear>, BLAKE3
├── crypto/             # 서명: P-256(Secure Enclave 규약), secp256k1, Ed25519
├── state/              # EIP-7864 바이너리 트리 + 증명 (geth V3 / ubt와 키·루트 일치 검증)
├── proving/            # Prover / Verifier 경계 (zkVM 백엔드는 스파이크 후 결정)
├── consensus/          # 합의 경계, 위원회 선출, FOCIL 포함 목록
└── da/                 # DA 경계 (로컬 구현, Celestia/이더리움 어댑터 예정)
legacy/                 # 기존 단일 노드 코드 (보안 수정 후 동결, 교체 대상)
docs/design, docs/research
```

## 라이선스

MIT 또는 Apache-2.0 듀얼 라이선스.
