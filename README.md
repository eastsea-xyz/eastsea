# Aether Node

> 맥이 있으면 누구나 검증자다. 내 맥이 직접 검증하는 지갑. 현재 **서로 다른 인터넷 회선의 검증자들이 공인 경로로 합의하는 네트워크와 맥 지갑 앱이 실제로 동작**합니다.

[![Legal Disclaimer](https://img.shields.io/badge/Legal-Disclaimer%20%26%20Terms-red.svg)](DISCLAIMER.md)

> [!IMPORTANT]
> Aether Node is experimental, non-commercial research software provided **"AS IS"**. It is not a production blockchain, has not been audited, and all tokens (AETH) and rewards are simulated test artifacts with **zero monetary value**. See [DISCLAIMER.md](DISCLAIMER.md).

## 지금 돌려 보기

```bash
scripts/demo.sh          # 검증자 4대를 띄우고 송금·계약·증명 검증·노드 간 일치까지 시연
scripts/devnet.sh stop   # 종료
```

검증자를 다른 맥(다른 인터넷 회선)에서 띄우기:

```bash
AETHER_LOCAL="1 2 3" scripts/devnet.sh start 4                     # 이 맥: 검증자 1~3
aether node --index 4 --validators 4 --port 9004 --rpc-port 8548 --data ~/aether-v4/data   # 다른 맥
```

포트 개방·VPN 없이 노드 ID만으로 서로 찾습니다. 위원회 키는 먼저 모든 검증자에서 동시에 `scripts/devnet.sh dkg 4`(다른 맥은 `aether dkg --index 4 --validators 4 --port 9004 --data …`)로 만들고, 출력된 identity(`network.json`)를 지갑 `apps/wallet/Resources/`에 넣습니다. 각 검증자 로그의 `validator links`가 경로(`direct <공인IP:포트>` 또는 `relay`)를 보여 줍니다.

rustup 툴체인 1.98.1이 필요합니다(`rust-toolchain.toml`). Homebrew rustc가 PATH 앞에 있으면 `export PATH="$HOME/.cargo/bin:$PATH"`.

직접 조작:

```bash
target/debug/aether dev-accounts                      # 제네시스에 자금이 있는 공개 개발 키 (가치 없음)
target/debug/aether send --from-dev 1 --to 0x… --value 1000 --wait
target/debug/aether balance 0x… --rpc http://127.0.0.1:8547   # 서버를 믿지 않고 Merkle 증명을 로컬 검증
target/debug/aether deploy --from-dev 2 --code <init code hex>
target/debug/aether blocks 10
```

## 현재 상태 (2026-09-26)

| 영역 | 지금 동작하는 것 | 아직 아닌 것 |
|---|---|---|
| 합의 | Commonware simplex BFT, 검증자 4대, 1초 블록, 1대 장애에도 진행. BLS12-381 임계 서명 인증서(검증자 수와 무관하게 131B, 그룹 공개키 하나로 검증), 키는 딜러 없는 DKG(`aether dkg`)로 생성, VRF 시드 기반 무작위 리더 | 재공유(검증자 교체), VRF 위원회 선출, 검증자 ed25519 키 로컬 생성(현재 devnet 공개 키) |
| 네트워크 | 검증자 간 연결·지갑 연결 모두 iroh QUIC(홀펀칭, 막히면 공개 릴레이). 주소는 BitTorrent Mainline DHT에서 노드 ID로 찾음. Tailscale/CGNAT·루프백 경로는 게시도 선택도 안 함. 다른 회선의 맥 2대(124.50.x ↔ 14.32.x)로 검증 | 검증자 목록 온체인 관리, 자체 릴레이 |
| 블록 전파·복구 | marshal: 확정 블록 순차 전달, 누락 보충, 디스크 아카이브, 재시작 시 재실행 복원 | 상태 스냅샷 동기화 |
| 실행 | revm으로 서명된 트랜잭션 실행(송금·계약 배포·호출), 가스 수수료, 모든 검증자가 재실행해 BAL·가스까지 일치 검사. 낙관적 병렬 실행(서명 검사 병렬, 충돌 시 재실행)으로 순차와 동일 결과 2배 안팎 | 트리 커밋(Poseidon2) 병렬화 |
| 상태 | EIP-7864 바이너리 트리(Poseidon2), geth V3 레퍼런스와 키·루트 일치, 포함·부재 증명. 확정 상태·영수증을 redb에 블록마다 원자적으로 저장, 재시작 시 루트 검증 후 체크포인트부터 재개 | 디스크 페이징(현재 상태 전체가 메모리), 스냅샷 동기화 |
| 서명 | P-256(Secure Enclave 규약), secp256k1, Ed25519 | 7702 위임 계정 |
| 클라이언트 검증 | CLI와 맥 지갑 앱이 확정 인증서(검증자 서명) + EIP-7864 증명으로 잔액을 로컬 검증. 지갑 키는 Secure Enclave | ZK 블록 증명(스파이크 후), iOS |
| MEV·검열 저항 | FOCIL식 포함 목록: 위원회가 서명한 대기 tx를 생산자가 넣지 않으면 투표 거부. 검열 검증자가 있어도 확정됨을 테스트로 확인 | 타임락 암호화 멤풀 |

`legacy/`는 이전 단일 노드 데모이며 교체되었습니다.

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
crates/
├── node/        # aether 바이너리: 검증자 노드(simplex + marshal) + JSON-RPC + CLI
├── execution/   # revm 실행, 트랜잭션 검증, BAL, 영수증, prove-gas
├── state/       # EIP-7864 바이너리 트리 + 증명
├── types/       # 봉투, 블록, BAL, 인증서, 증명, 매니페스트
├── hash/        # Poseidon2<KoalaBear>, BLAKE3
├── crypto/      # P-256, secp256k1, Ed25519
├── consensus/   # 위원회 선출, 포함 목록 규칙
├── proving/     # Prover / Verifier 경계
└── da/          # DA 경계
scripts/         # devnet.sh, demo.sh
legacy/          # 이전 데모 (동결)
docs/design, docs/research
```

## 라이선스

MIT 또는 Apache-2.0 듀얼 라이선스.
