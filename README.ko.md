# Aether Node

[English](README.md) · **한국어** · [中文](README.zh-CN.md) · [日本語](README.ja.md) · [Tiếng Việt](README.vi.md) · [Español](README.es.md)

> 어떤 Mac이든 validator가 될 수 있고, 내 지갑은 내 Mac이 직접 검증합니다. 현재 서로 다른 가정용 인터넷 회선에 있는 validator들이 공용 경로를 통해 합의에 도달하며, Mac과 iPhone 지갑 앱이 이 네트워크에서 동작합니다.

[![Legal Disclaimer](https://img.shields.io/badge/Legal-Disclaimer%20%26%20Terms-red.svg)](DISCLAIMER.md)

> [!IMPORTANT]
> Aether Node는 **"있는 그대로(AS IS)"** 제공되는 실험적·비상업적 연구용 소프트웨어입니다. 프로덕션 블록체인이 아니며 감사(audit)를 받지 않았습니다. 모든 토큰(AETH)과 보상은 테스트용 산출물로 **금전적 가치가 전혀 없습니다**. [DISCLAIMER.md](DISCLAIMER.md)를 참고하십시오.

## 개요

- **Mac 우선.**
  - 지갑 키는 Secure Enclave에 보관되며, 모든 결제마다 Touch ID를 요구합니다. 시드 문구(seed phrase)는 없습니다.
  - 다른 Apple 기기 한 대를 복구 키로 등록할 수 있습니다.
- **신뢰하지 말고 검증하십시오.**
  - 지갑은 각 잔액을 기기에서 직접 확인합니다. validator 위원회의 BLS threshold 서명 하나와 EIP-7864 상태 증명을 사용합니다.
  - 서버의 말을 그대로 믿지 않습니다.
- **포트 개방도, VPN도 필요 없습니다.**
  - validator와 지갑은 BitTorrent Mainline DHT에서 node ID로 서로를 찾습니다.
  - iroh QUIC으로 hole punching을 통해 연결하고, 실패하면 relay로 전환합니다.
- **AI 에이전트도 고려해 만들었습니다.**
  - `aether-agent`는 Claude Code, Codex, Antigravity, OpenClaw, Hermes 또는 모든 MCP 클라이언트에 지갑을 제공합니다.
  - 키는 Secure Enclave에 있고, 지출 한도는 사용자만 Touch ID로 변경할 수 있습니다.

## 사용해 보기

```bash
scripts/demo.sh          # validator 4개를 시작하고 송금, 컨트랙트, 증명 검사, 노드 간 합의를 보여 줍니다
scripts/devnet.sh stop   # 중지합니다
```

rustup 툴체인 1.98.1(`rust-toolchain.toml`)이 필요합니다. PATH에서 Homebrew의 rustc가 먼저 잡힌다면 `export PATH="$HOME/.cargo/bin:$PATH"`를 실행하십시오.

### 지갑 앱 (macOS, iOS)

```bash
scripts/build-wallet.sh           # macOS 앱
scripts/build-wallet.sh ios-sim   # iOS 시뮬레이터
```

- **Simple 모드(기본값):**
  - 잔액과 잔액 차트, 그리고 보내기, 받기(QR), 테스트 토큰 받기가 있는 홈 화면.
  - 활동 내역, 네트워크 상태, 복구 설정 페이지.
- **Developer 모드:** 증명, state root, 원시 로그와 블록. 왼쪽 위의 스위치로 모드를 전환합니다.

### AI 에이전트용 지갑

```bash
scripts/build-agent.sh --install   # ~/.local/bin/aether-agent
aether-agent init                  # 사용자가 한 번만: 키와 기본 한도를 생성합니다 (Touch ID)
aether-agent setup all --apply     # 설치된 모든 에이전트 도구에 MCP 서버 "aether"를 등록합니다
```

- **도구:** status, wallet, balance, send, pay_many(트랜잭션 하나), receipt, history.
- **기본 한도:** 결제당 1 AETH, 24시간당 10 AETH. `aether-agent policy set`으로 변경할 수 있으며, 이때 Touch ID를 요구합니다.
- **변조 검사:** 에이전트가 정책 파일을 수정하면 지출이 중단됩니다. 지출 로그는 서명되어 있으며 온체인 nonce와 교차 검증됩니다.
- 자세한 내용: [AGENTS.md](AGENTS.md) 및 [skill 파일](agents/skills/aether-wallet/SKILL.md).

### 실제 네트워크 운영

각 validator 머신에서 각자의 키를 생성합니다. 그런 다음 공개 부분을 모으고, 함께 키 세리머니를 진행한 뒤 노드를 시작합니다.

```bash
aether keygen --data ~/aether/v1                         # 각 머신에서 실행하며, 비밀 키는 그 머신에만 남습니다
aether network v1.pub.json v2.pub.json … > network.json  # 공개 부분만 사용하며, 모두에게 배포합니다
AETHER_NETWORK=network.json scripts/devnet.sh dkg 4      # 다른 머신: aether dkg --network network.json --port … --data …
AETHER_NETWORK=network.json scripts/devnet.sh start 4    # 다른 머신: aether node --network network.json …
# 지갑: <data>/network.json(node ID + 위원회 키)을 apps/wallet/Resources/에 복사합니다
```

위원회 키는 validator가 바뀌어도 유지됩니다. 새 validator 집합으로 옮기려면 `aether reshare`를 사용하십시오.

### 명령줄

```bash
target/debug/aether dev-accounts                               # genesis에서 자금이 할당된 공개 테스트 키 (가치 없음)
target/debug/aether send --from-dev 1 --to 0x… --value 1000 --wait
target/debug/aether balance 0x… --rpc http://127.0.0.1:8547   # 신뢰하지 않고 증명으로 로컬에서 검증합니다
target/debug/aether batch --from-dev 1 --to 0xA,0xB --value 1  # 여러 결제를 서명 하나로 처리합니다
target/debug/aether blocks 10
```

## 현황 (2026-09-26)

| 영역 | 현재 동작 | 아직 미지원 |
|---|---|---|
| 합의 | Commonware simplex BFT: validator 4개, 1초 블록, validator 하나가 다운되어도 계속 진행. BLS12-381 threshold 인증서(131 B, 그룹 키 하나로 검증). 키는 로컬에서 생성하며, 위원회 키는 dealerless DKG로 생성. reshare를 통한 validator 교체. VRF 시드 기반 무작위 리더 | 온체인 위원회 변경, VRF 위원회 선출 |
| 네트워크 | validator와 지갑 사이의 iroh QUIC 연결(hole punching 또는 relay). Mainline DHT에서 node ID로 주소 탐색. Tailscale, CGNAT, loopback 경로는 사용하지 않음. 서로 다른 두 ISP의 Mac으로 테스트함 | 온체인 validator 목록, 자체 relay |
| 실행 | revm: 송금, 컨트랙트 배포 및 호출. 모든 validator가 각 블록을 재실행하며, 블록 접근 목록(BAL) 및 gas와 일치해야 함. optimistic 병렬 실행은 순차 실행과 같은 결과를 냄 | 병렬 트리 커밋 |
| 수수료 | 실행과 증명에 대한 별도의 base fee를 EIP-4844 방식으로 조정. 실행 base fee는 소각되고, 증명 수수료는 prover escrow로 이동. 팁은 proposer 60%, prover escrow 20%, 소각 20%로 분배 | 증명된 chunk 단위 escrow 청구 |
| 상태 | EIP-7864 binary tree(Poseidon2)로, 키와 root가 geth 레퍼런스와 일치. 포함 증명 및 부재 증명. 블록마다 redb에 원자적으로 저장되며, 재시작 후 체크포인트에서 재개 | 디스크 페이징, 스냅샷 동기화 |
| 계정 | P-256(Secure Enclave), secp256k1, Ed25519. `AetherAccount`에 대한 EIP-7702 위임으로 서명 하나로 일괄 결제 가능. 두 번째 기기의 Secure Enclave 키를 복구 키로 사용 가능 | 세션 키, 다중 guardian, 시간 잠금 복구 |
| 클라이언트 | Mac 및 iOS 지갑(Simple 및 Developer 모드), CLI, `aether-agent`(MCP) 모두 잔액을 로컬에서 검증 | 클라이언트 내 ZK 블록 증명, TestFlight |
| 검열 저항성 | FOCIL 방식의 inclusion list: 목록에 있는 트랜잭션을 누락한 블록에는 validator가 투표를 거부 | 암호화된 mempool |
| 증명 (spike) | Jolt zkVM이 실제 Aether 블록을 증명하며, root가 네이티브 실행과 일치. Mac 한 대당 시간당 약 270개 트랜잭션. 증명은 체인보다 뒤따라감 | Metal 백엔드, 체크포인트 증명 |

`legacy/`는 이전의 단일 노드 데모이며 현재는 대체되었습니다.

## 설계

- [구현 설계](docs/design/00-overview.md): 정체성, 결정 사항 D1–D18, 각 계층의 설계
- [리서치](docs/research/): 각 결정의 근거 자료. [tokenomics 2026](docs/research/tokenomics-2026.md) 포함
- [Spike 결과](docs/research/spike-2026-10.md)

## 테스트

```bash
cargo test --workspace
cargo test -p aether-state --test eip7864_compat   # EIP-7864 레퍼런스와 교차 검증합니다
cargo test -p aether-execution --test fees         # 수수료 분배와 가치 보존을 확인합니다
```

## 구성

```
crates/
├── node/        # aether binary: validator (simplex + marshal), JSON-RPC, CLI, DKG
├── execution/   # revm execution, tx validation, BAL, fees, receipts, prove gas
├── state/       # EIP-7864 binary tree and proofs
├── types/       # envelopes, blocks, BAL, certificates, proofs
├── light/       # light client: committee key, certificate checks
├── net/         # iroh links, Mainline DHT discovery
├── ffi/         # wallet core for Swift (UniFFI)
├── hash/ crypto/ consensus/ proving/ da/
apps/
├── wallet/      # macOS and iOS wallet (SwiftUI)
└── agent/       # aether-agent: MCP server and JSON CLI for AI agents
agents/skills/   # SKILL.md for agent tools
contracts/       # AetherAccount (EIP-7702 batch + recovery)
spike/           # zkVM proving experiments
scripts/         # devnet.sh, demo.sh, build-wallet.sh, build-agent.sh
docs/design, docs/research
```

## 라이선스

MIT 또는 Apache-2.0 이중 라이선스로 배포됩니다.
