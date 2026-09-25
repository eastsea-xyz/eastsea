# 01. Workspace 구조와 의존성

## 크레이트

```
aether/
├── Cargo.toml                 # workspace, [workspace.dependencies]로 버전 단일 고정
├── crates/
│   ├── types/                 # 봉투, 블록, BAL, 인증서, 매니페스트 (no_std 가능)
│   ├── hash/                  # Hasher trait + Poseidon2⟨KoalaBear⟩, BLAKE3 백엔드
│   ├── crypto/                # 서명자 trait, P-256, secp256k1, ed25519, BLS 래핑
│   ├── state/                 # 바이너리 SMT, EIP-7864 키, NOMT repository
│   ├── execution/             # revm 래핑, BAL, 정적 DAG Block-STM, prove-gas Inspector
│   ├── proving/               # Prover/Verifier trait, PCS trait, 청크·재귀, zkVM 게스트 빌드
│   ├── consensus/             # Commonware simplex 통합, 위원회 선출, 포크 선택
│   ├── da/                    # DA trait, Celestia(Lumina) 어댑터, 이더리움 blob 어댑터
│   ├── network/               # iroh, Pkarr, 피어 관리, gossip
│   ├── history/               # 매니페스트, 다중 미러 다운로더, 청크 시딩
│   ├── rpc/                   # eth JSON-RPC (alloy)
│   ├── node/                  # 파이프라인 조립, 역할(검증노드/검증자/증명기), 설정
│   ├── ffi/                   # UniFFI 인터페이스 (지갑용)
│   └── advisor/               # AI 자문 (tract), feature flag
├── guest/                     # zkVM 게스트 프로그램 (RV64IM, 별도 타깃)
│   └── block-exec/
├── apps/
│   ├── aether-node/           # CLI 데몬 (기존 src/bin/node.rs 대체)
│   ├── aether-bench/          # 벤치 실행기
│   └── Aether.app/            # SwiftUI (Xcode 프로젝트)
├── specs/
│   └── consensus.qnt          # Quint 명세
├── tests/
│   ├── differential/          # 순차 vs Block-STM vs grevm
│   └── simulation/            # 결정적 시뮬레이션 시나리오
└── docs/
```

기존 `src/`는 0단계에서 보안 수정 후 동결하고, 1단계에서 위 구조로 이전한다.
`src/web/dashboard.html`은 `apps/aether-node/web/`로 옮겨 엔지니어 모드로 유지한다.

## 의존성 (버전 고정)

| 크레이트 | 버전 | 용도 | 비고 |
|---|---|---|---|
| revm | =43.0.1 | EVM | 43.0.0 selfdestruct 회귀 회피 |
| alloy-primitives, alloy-evm | 1.7.x / 0.39 | 타입, 블록 실행 glue | EIP-8037 상태 가스 포함 |
| commonware-consensus, -p2p, -broadcast, -cryptography, -runtime | =2026.9.0 | 합의, 검증자 통신, BLS, 결정적 런타임 | 월간 릴리스, 파괴적 변경 → 고정 |
| nomt | =1.0.4 | 상태 DB | io_uring은 Linux, macOS는 fallback |
| p3-poseidon2, p3-koala-bear, p3-mersenne-31, p3-blake3 | Plonky3 최신 태그 | 해시 백엔드 | |
| iroh, iroh-blobs, iroh-relay | =1.2.x | 네트워크, 청크 전송 | |
| pkarr | =8.0.x | 서명된 부트노드 목록 | |
| p256, k256, ed25519-dalek | 0.14 / 0.14 / 3.0 | 서명 | ed25519 3.x는 2.x와 비호환 |
| cryptokit-rs | 0.3 | Secure Enclave 브리지 | macOS 26 SDK |
| uniffi | 0.29+ | Rust↔Swift | pre-1.0 |
| lumina (celestia) | 최신 | DA 라이트 노드 | wasm 가능 |
| tract | 최신 | 자문 모델 추론 | CPU, 결정적 |
| proptest, turmoil | 최신 | 테스트 | |
| grevm | =2.2.6 (git) | 차등 테스트 정답지만 | revm 40 기준이라 별도 dev-dep |

zkVM 크레이트(jolt / stwo / risc0)는 스파이크 결과로 확정. `proving/`는 feature flag로 백엔드를 선택한다.

## 금지 의존성

aptos-core (라이선스), firewood (라이선스), ferveo (GPL), MonadDB (GPL, C++), Blockscout 현재 버전 (상용 라이선스).

## 빌드 타깃

- 호스트: aarch64-apple-darwin (1급), x86_64-unknown-linux-gnu (CI, 비교)
- 게스트: riscv64imac-unknown-none-elf (zkVM별 툴체인)
- 지갑 검증기: aarch64-apple-darwin, aarch64-apple-ios, wasm32-unknown-unknown

## 프로필

```toml
[profile.release]
lto = "fat"
codegen-units = 1
[profile.bench]
inherits = "release"
debug = true            # 프로파일러용
```
