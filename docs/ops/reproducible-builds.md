# 재현 가능한 빌드

## 왜

메인넷 갭 **G5**: 증명 프로그램 ID와 노드·앱 바이너리가 **빌드한 디렉터리에 따라** 달라지면, 제3자가 "이 릴리스는 이 소스에서 나왔다"를 확인할 수 없다. 노드는 증명 프로그램 ID(`AETHER_PROVER_PROGRAM`)로 검증할 프로그램을 고정하므로, 같은 소스가 다른 ID를 내면 합의 자체가 흔들린다. 그래서 릴리스에 쓰는 모든 Rust 빌드를 **경로·시각과 무관**하게 만들고, 그 사실을 스크립트로 증명한다.

핵심 원칙은 재현 가능 빌드 표준을 따른다: 한 타임스탬프(`SOURCE_DATE_EPOCH`), 경로 재매핑(`--remap-path-prefix`), 고정 툴체인(`rust-toolchain.toml` + `--locked`), 결정적 아카이브(`ZERO_AR_DATE=1`), 단일 코드젠 유닛(`codegen-units = 1`).

배경 조사는 [docs/research/repro-builds-2026.md](../research/repro-builds-2026.md).

## 3계층 모델

| 계층 | 대상 | 재현 수준 | 검증 |
|---|---|---|---|
| **Tier 1** | `aether`(노드), `libaether_ffi.a`, `aether-prover`·증명 프로그램 ID, 브라우저 확장 ZIP | **비트 단위 일치** | 배포본과 제3자 빌드의 SHA-256 대조 |
| **Tier 2** | macOS `AetherWallet.app` 내부 Mach-O | **서명 제거 후 비트 일치 + `LC_UUID` 일치** | `codesign --remove-signature` 뒤 해시, `dwarfdump --uuid` |
| **Tier 3** | iOS App Store 배포본 | **외부 검증 불가** | (아래 "재현되지 않는 것" 참고) |

## 빌드 환경 (`scripts/repro-env.sh`)

릴리스에 쓰는 모든 빌드 스크립트가 이 파일을 source 한 뒤 `aether_repro_rustflags`를 부른다.

| 설정 | 값 | 이유 |
|---|---|---|
| `SOURCE_DATE_EPOCH` | `git log -1 --pretty=%ct` (`.git`이 없으면 `1767225600` = 2026-01-01Z) | 아카이브·패키지의 타임스탬프를 커밋 시각으로 고정 |
| `ZERO_AR_DATE=1` | 항상 | Apple `ar`/`libtool`이 `.a` 멤버에 박는 빌드 시각 제거 (`libaether_ffi.a`) |
| `AETHER_REMAP_FLAGS` | 체크아웃→`/aether-node`, `~/.cargo/registry/src`·`git/checkouts`→`/cargo`, `~/.rustup`→`/rustup`, 타깃 디렉터리→`/aether-target` | 패닉 위치·`file!()`·디버그 심볼에 빌더의 절대 경로가 남지 않게 |
| `--locked` | 모든 `cargo build` | `Cargo.lock`을 동결 (툴체인은 `rust-toolchain.toml`의 `1.98.1`) |
| `codegen-units = 1` | `Cargo.toml` `[profile.release]`(루트·`apps/prover`) | LLVM이 섹션 배치를 스레드 순서에 따라 바꾸지 않게 |

호스트 링크에는 추가로 `-C link-arg=-Wl,-reproducible`을 준다(`build-wallet.sh`·`build-agent.sh`·`testnet-*.sh`·`repro-check.sh`). wasm 링크에는 주지 않는다(wasm-ld가 모를 수 있다).

증명 프로그램 ID는 **게스트 ELF의 SHA-256**이다(`apps/prover/src/program.rs`). 게스트는 `apps/prover/build-guest.sh`가 jolt CLI로 빌드하며, 여기서도 같은 remap과 `SOURCE_DATE_EPOCH`/`ZERO_AR_DATE`를 적용한다. 루트 워크스페이스의 `codegen-units`는 게스트에 닿지 않으므로 `apps/prover/Cargo.toml`에 따로 둔다.

## 검사 스크립트

### Tier 1: `scripts/repro-check.sh`

체크아웃을 서로 **다른 길이의 두 디렉터리**(`a`, `bbbbbbbbbb`)로 복사하고 각각 콜드 빌드한 뒤 SHA-256을 비교한다. 두 번째 디렉터리 이름을 길게 둔 이유는 경로가 바이너리에 새면 길이 차이로 드러나게 하기 위해서다.

```bash
scripts/repro-check.sh                    # node, ffi, prover, extension 전부
scripts/repro-check.sh node extension      # 골라서
```

| 아티팩트 | 비교값 |
|---|---|
| `node` | `target/release/aether` |
| `ffi` | `target/release/libaether_ffi.a` |
| `prover-program` | `scripts/prover-program.sh`가 내는 프로그램 ID |
| `prover-binary` | `target/release/aether-prover` |
| `extension` | `dist/aether-extension-<version>.zip` |

환경변수: `AETHER_REPRO_WORK`(두 트리를 둘 곳, 기본 `$TMPDIR`), `AETHER_REPRO_KEEP=1`(남겨 두기), `AETHER_REPRO_SERIAL=1`(한 쪽씩), `AETHER_PROFILE`(기본 `release`). Jolt 포크나 `jolt` CLI가 없으면 prover는 건너뛴다. 두 번의 콜드 빌드라 오래 걸리고 타깃 디렉터리 두 개분의 디스크가 필요하다.

### Tier 2: `scripts/repro-app-check.sh`

`.app`은 서명(Xcode가 붙이는 ad-hoc 서명 + Developer ID + 공증 티켓 스테이플) 때문에 통째로는 비트가 같아질 수 없다. 그래서 번들 안의 모든 Mach-O를 복사해 `codesign --remove-signature`로 서명을 떼고 SHA-256과 `LC_UUID`를 비교한다.

```bash
scripts/repro-app-check.sh                 # 앱을 두 번 빌드해서 비교
scripts/repro-app-check.sh A.app B.app     # 이미 빌드한 번들 두 개 비교
```

`LC_UUID`가 같으면 링커가 같은 이미지를 냈다는 뜻이고, 서명 제거 해시가 같으면 코드·데이터가 같다는 뜻이다. 빌드에는 Developer ID 인증서와 Jolt 포크가 필요하다. **서명·공증은 하지 않는다** — 서명이 붙은 산출물을 비교만 한다.

### 단위 테스트: `scripts/test-repro-scripts.sh`

cargo/Xcode 없이 빠르게 돈다: `repro-env.sh`가 에폭·`ZERO_AR_DATE`·remap을 맞게 내는지, `deterministic-zip.py`가 같은 내용을 같은 바이트로 싸는지(`.test.` 제외 포함), `repro-app-check.sh`가 서명만 다른 두 번들을 "재현됨", 코드가 다른 두 번들을 "다름"으로 판정하는지.

```bash
scripts/test-repro-scripts.sh
scripts/test-reproducibility.sh   # ZIP·Mach-O 개념 검증(기존)
```

## 확장 ZIP 패키징 (`scripts/deterministic-zip.py`)

`zip` 명령은 파일 mtime·순회 순서·모드를 그대로 기록해서, 같은 파일을 두 번 싸도 두 해시가 나온다. 대신 `deterministic-zip.py`가 `SOURCE_DATE_EPOCH`(UTC) 시각, 정렬된 엔트리, `0644` 모드로 싼다. `build-extension.sh --zip`이 이 스크립트를 쓴다.

## 재현되지 않는 것 (정직하게)

- **macOS `.app` 번들 자체.** 코드서명은 개발자 개인키와 Apple TSA 타임스탬프에 의존하고, 공증 티켓을 스테이플하면 번들이 바뀐다. 그래서 번들 통짜 해시는 재현 대상이 아니다. Tier 2(서명 제거 + `LC_UUID`)로만 검증한다.
- **iOS App Store 배포본.** Apple이 FairPlay DRM(`LC_ENCRYPTION_INFO`의 `cryptid=1`)으로 암호화하고, App Slicing으로 기기별로 다시 패키징하며, Apple 인증서로 재서명한다. 제3자가 App Store 바이너리를 같은 해시로 재생성하는 것은 **수학적으로 불가능**하다([Wallet Scrutiny 방법론](https://walletscrutiny.com/methodology/)). 대신 배포 전 **미서명 IPA/xcarchive**의 SHA-256과 `LC_UUID`를 릴리스 노트에 공개해 대조한다.
- **툴체인·SDK 버전.** 같은 Xcode/rustc/zlib이어야 같은 바이트가 나온다. `rust-toolchain.toml`은 Rust를 고정하지만 Xcode 버전은 아직 고정하지 않는다(Tier 2 검증에는 링커 버전이 영향을 준다). `-Wl,-reproducible`은 이 저장소가 확인한 Xcode 26.6(ld-1267)에서 동작한다 — 더 오래된 링커는 이 플래그를 모를 수 있다.
- **Jolt 포크 위치.** `apps/prover`는 Jolt·Akita를 `/Volumes/workspace/aether-jolt`에서 절대 경로로 링크한다(포크는 별도 저장소라 이 저장소가 고정하지 않는다). 경로는 `/jolt`로 remap되므로 빌드 디렉터리는 영향을 주지 않지만, 포크 리비전이 다르면 증명 프로그램 ID가 달라진다. 릴리스마다 포크 커밋을 함께 기록한다.

## 릴리스 절차

1. `scripts/test-repro-scripts.sh` (빠른 확인).
2. `scripts/repro-check.sh all` — 네 아티팩트가 두 디렉터리에서 같은 바이트로 나오는지. 통과한 해시를 릴리스 노트에 적는다.
3. `scripts/repro-app-check.sh` — 앱을 두 번 빌드해 서명 제거 Mach-O와 `LC_UUID` 대조.
4. iOS는 미서명 아카이브 해시 + `LC_UUID`만 공개하고 "App Store 바이너리는 검증 불가"를 명시한다.
