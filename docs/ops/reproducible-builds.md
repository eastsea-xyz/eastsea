# 재현 가능한 빌드

## 왜

메인넷 갭 **G5**: 증명 프로그램 ID와 노드·앱 바이너리가 **빌드한 디렉터리에 따라** 달라지면, 제3자가 "이 릴리스는 이 소스에서 나왔다"를 확인할 수 없다. 노드는 증명 프로그램 ID(`AETHER_PROVER_PROGRAM`)로 검증할 프로그램을 고정하므로, 같은 소스가 다른 ID를 내면 합의 자체가 흔들린다. 그래서 릴리스에 쓰는 모든 Rust 빌드를 **경로·시각과 무관**하게 만들고, 그 사실을 스크립트로 증명한다.

핵심 원칙은 재현 가능 빌드 표준을 따른다: 한 타임스탬프(`SOURCE_DATE_EPOCH`), 경로 재매핑(`--remap-path-prefix`), 고정 툴체인(`rust-toolchain.toml` + `--locked`), 결정적 아카이브(`ZERO_AR_DATE=1`), 단일 코드젠 유닛(`codegen-units = 1`).

배경 조사는 [docs/research/repro-builds-2026.md](../research/repro-builds-2026.md).

## 3계층 모델

| 계층 | 대상 | 재현 수준 | 검증 |
|---|---|---|---|
| **Tier 1** | `aether`(노드), `libaether_ffi.a`, `aether-prover`·증명 프로그램 ID, 브라우저 확장 ZIP | **비트 단위 일치** (노드·사이드카는 `LC_UUID` 재작성 뒤) | 배포본과 제3자 빌드의 SHA-256 대조 |
| **Tier 2** | macOS `AetherWallet.app` 내부 Mach-O | **서명 제거 + `LC_UUID`를 0으로 비운 뒤 비트 일치** | `codesign --remove-signature` 뒤 해시, `macho-uuid.py`, `dwarfdump --uuid` |
| **Tier 3** | iOS App Store 배포본 | **외부 검증 불가** | (아래 "재현되지 않는 것" 참고) |

## 빌드 환경 (`scripts/repro-env.sh`)

릴리스에 쓰는 모든 빌드 스크립트가 이 파일을 source 한 뒤 `aether_repro_rustflags`를 부른다.

| 설정 | 값 | 이유 |
|---|---|---|
| `SOURCE_DATE_EPOCH` | `git log -1 --pretty=%ct` (`.git`이 없으면 `1767225600` = 2026-01-01Z) | 아카이브·패키지의 타임스탬프를 커밋 시각으로 고정 |
| `ZERO_AR_DATE=1` | 항상 | Apple `ar`/`libtool`이 `.a` 멤버에 박는 빌드 시각 제거 (`libaether_ffi.a`) |
| `AETHER_REMAP_FLAGS` | 체크아웃→`/aether-node`, `~/.cargo/registry/src`·`git/checkouts`→`/cargo`, `~/.rustup`→`/rustup`, 타깃 디렉터리→`/aether-target` | 패닉 위치·`file!()`·디버그 심볼에 빌더의 절대 경로가 남지 않게 |
| 위 remap의 두 표기 | 각 경로를 **주어진 그대로**와 **물리 경로**(macOS `/tmp`→`/private/tmp`) 두 번 모두 remap | cargo는 어떤 경로는 정규화해서, 어떤 경로는 그대로 rustc에 넘긴다. 한쪽만 remap하면 새는 경로가 생기고, 타깃 디렉터리가 이미 있느냐에 따라 플래그 목록이 달라져(전부 다시 빌드) 웜/콜드 빌드가 갈린다 |
| `--locked` | 모든 `cargo build` | `Cargo.lock`을 동결 (툴체인은 `rust-toolchain.toml`의 `1.98.1`) |
| `codegen-units = 1` | `Cargo.toml` `[profile.release]`(루트·`apps/prover`) | LLVM이 섹션 배치를 스레드 순서에 따라 바꾸지 않게 |

호스트 링크에는 추가로 `-C link-arg=-Wl,-reproducible`을 준다(`build-wallet.sh`·`build-agent.sh`·`testnet-*.sh`·`repro-check.sh`). wasm 링크에는 주지 않는다(wasm-ld가 모를 수 있다).

증명 프로그램 ID는 **게스트 ELF의 SHA-256**이다(`apps/prover/src/program.rs`). 게스트는 `apps/prover/build-guest.sh`가 jolt CLI로 빌드하며, 여기서도 같은 remap과 `SOURCE_DATE_EPOCH`/`ZERO_AR_DATE`를 적용한다. 루트 워크스페이스의 `codegen-units`는 게스트에 닿지 않으므로 `apps/prover/Cargo.toml`에 따로 둔다.

### 증명 프로그램 ID와 고정 경로 (`scripts/guest-stage.sh`)

**remap만으로는 부족하다.** cargo는 `-C metadata`에 path 의존성의 **경로 자체**를 해시한다(`PackageId::stable_hash`: 그 경로가 그 빌드의 워크스페이스 루트 아래에 있으면 루트 기준 상대 경로, 아니면 절대 경로). `apps/prover`는 자기 워크스페이스 루트이고(`[workspace] members = ["guest"]`) Aether 크레이트는 그 위에 있으므로, 체크아웃 경로가 메타데이터 해시 → 심볼 이름 → (Jolt가 내는 코드 배치를 거쳐) 게스트 ELF의 `.text`까지 들어간다. 실측: 같은 소스의 두 체크아웃이 낸 ELF는 크기가 같고 `.text` 안에서 **52,068바이트**가 달랐다(순열이 아니라 국소 차이 — 16바이트 창의 99.75%가 상대 파일에도 있고 98%는 같은 오프셋에서 일치). `--remap-path-prefix`는 rustc가 **보고하는** 경로를 바꾸지, cargo가 **해시하는** 경로를 바꾸지 못한다.

그래서 호스트 사이드카와 게스트 **둘 다** 고정 경로 **스테이지**를 거쳐 빌드한다(`scripts/guest-stage.sh`):

- 기본 경로는 `/tmp/aether-guest-stage`이고 `AETHER_GUEST_STAGE`로 바꾼다. 스테이지는 체크아웃을 가리키는 심링크로 채운 **실제 디렉터리**다. `apps/prover`만은 심링크가 아니라 실제 디렉터리인데, 그 경로가 워크스페이스 루트로서 메타데이터의 기준이기 때문이다 — 여기가 심링크면 그 경로를 해석하는 도구(Jolt CLI가 워크스페이스를 훑는다)가 체크아웃 경로를 되돌려 놓는다.
- 빌드 산출물은 스테이지가 아니라 체크아웃에 남는다(`target`·`target-guest`는 체크아웃을 가리키는 심링크). 스테이지는 cargo가 **읽는** 입력만 고정한다.
- 스테이지는 한 번에 한 체크아웃만 가리키는 공유 상태라, 쓰는 빌드는 `<스테이지>.lock`으로 직렬화된다. `build.rs`가 부르는 중첩 `build-guest.sh`는 `AETHER_GUEST_STAGE_LOCKED`로 부모의 락을 물려받아 기다리지 않고(데드락 방지), 부모의 락을 풀지도 않는다. 소유 프로세스가 죽은 락은 회수하고, 살아 있지만 멈춘 소유자를 위해 기다리는 쪽은 `AETHER_GUEST_STAGE_LOCK_WAIT`초(기본 2시간) 뒤에 포기한다(0이면 무한 대기). `repro-check.sh`의 두 트리는 prover 단계에서만 이 락으로 직렬화된다(노드·FFI·확장은 계속 병렬).
- **스테이지 안에서 시작한 빌드는 스테이지를 다시 채우지 않는다.** `build.rs`가 부르는 중첩 `build-guest.sh`는 cargo가 패키지 루트로 스테이지 경로를 주므로 `../..`이 체크아웃이 아니라 스테이지다(스테이지 안에서 손으로 돌린 경우도 같다). 그 경로로 populate하면 모든 링크가 자기 자신을 가리켜(`crates -> crates`) cargo가 워크스페이스 루트를 못 찾는다 — 그래서 `aether_guest_stage_populate`는 루트가 스테이지 자신이면 아무것도 하지 않고, `build-guest.sh`는 그 경우 `repo`를 비워 remap에도 넣지 않는다.
- **모든 빌더가 같은 `AETHER_GUEST_STAGE` 값을 써야 한다.** 스테이지 경로는 프로그램 ID의 일부다.

게스트는 어떤 경로에서 빌드해도(맨손 `cargo build` 포함) 스테이지를 지나므로 프로그램 ID가 안정적이다. 사이드카 바이너리 자체의 비트 일치는 스테이지 안에서 호스트까지 빌드하는 `scripts/prover-program.sh`로 확인한다.

**주의: 프로그램 ID가 한 번 바뀐다.** 스테이지를 넣으면 ID가 (체크아웃 경로에서 유래한 값에서) 고정 경로의 값으로 옮겨 간다. 릴리스마다 `scripts/prover-program.sh`가 내는 값을 그대로 다시 적고, 그 값이 박힌 노드(`AETHER_PROVER_PROGRAM`)와 사이드카를 함께 배포한다.

### 링커의 `LC_UUID` (`aether_repro_fix_uuid`)

`-reproducible`만으로는 부족하다. 링커는 `LC_UUID` 로드 커맨드와 그 16바이트를 덮는 ad-hoc 서명 페이지를 남기는데, 그 값은 링크에 들어간 입력(출력 파일 이름, dylib의 출력 경로, 오브젝트·`rlib` 파일들)을 따라간다. 실측: 노드를 두 디렉터리에서 빌드하면 바이너리가 **정확히 48바이트** — `LC_UUID` 16바이트와 그 위의 서명 페이지 32바이트 — 만 달랐고, 나머지는 비트 단위로 같았다.

이걸 고정하는 링커 플래그는 없다. `ld -help`에 UUID 관련 옵션이 없고(확인함), `-no_uuid`는 **실행 자체를 막는다**: macOS 26(Darwin 25)의 dyld는 `LC_UUID`가 없는 Mach-O를 거부한다(`dyld: missing LC_UUID load command`, SIGABRT, 종료 코드 134). RUSTFLAGS에 넣으면 cargo가 호스트에서 실행하는 **빌드 스크립트 바이너리**부터 이 이유로 죽어 빌드 전체가 실패한다(이 저장소에서 실제로 겪었다). 노드도 실행돼야 하므로 릴리스 바이너리에는 쓸 수 없다.

그래서 링크 **뒤에** 다시 쓴다:

```bash
aether_repro_fix_uuid target/release/aether     # scripts/repro-env.sh
```

1. `codesign --remove-signature` — ad-hoc 서명이 UUID를 덮으므로 먼저 뗀다.
2. `scripts/macho-uuid.py rebuild` — UUID를 0으로 비우고, 그 상태의 **파일 전체 SHA-256**을 UUID 자리에 쓴다.
3. `codesign --force --sign - --identifier <출력 파일 이름>` — 서명을 다시 붙인다(arm64는 서명이 깨진 바이너리도 실행하지 않는다). 식별자는 링커가 썼을 값과 같은 출력 파일 이름이다.

결과는 빌드 ID다: 같은 코드는 같은 UUID, 다른 코드는 다른 UUID. 여러 번 불러도 바이트가 변하지 않으므로(한 세션에서 여러 릴리스 스크립트가 같은 바이너리를 지나간다) `build-wallet.sh`(노드)·`build-agent.sh`·`testnet-reset.sh`·`testnet-upgrade.sh`·`prover-program.sh`(사이드카)·`repro-check.sh`가 모두 부담 없이 부른다. 대가: UUID가 링크 시점이 아니라 이 단계에서 정해지므로 `dsymutil`이 링크 직후 만든 dSYM과는 짝이 맞지 않는다 — 릴리스 프로파일은 디버그 심볼을 만들지 않아 잃을 것이 없다(심볼이 필요하면 `aether_repro_fix_uuid` 뒤에 `dsymutil`을 돌린다).

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

환경변수: `AETHER_REPRO_WORK`(두 트리를 둘 곳, 기본 `$TMPDIR`), `AETHER_REPRO_KEEP=1`(남겨 두기), `AETHER_REPRO_SERIAL=1`(한 쪽씩), `AETHER_PROFILE`(기본 `release`). Jolt 포크나 `jolt` CLI가 없으면 prover는 건너뛴다. prover 단계는 두 트리가 같은 스테이지(`AETHER_GUEST_STAGE`)를 쓰므로 스테이지 락으로 직렬화되어 한 쪽씩 돈다(나머지는 병렬). 두 번의 콜드 빌드라 오래 걸리고 타깃 디렉터리 두 개분의 디스크가 필요하다.

#### `libaether_ffi.a`와 uniffi_bindgen

`.a`가 두 디렉터리에서 달라지던 원인은 이 저장소의 코드가 아니라 **`uniffi_bindgen`**이었다. uniffi의 `cli` 기능을 켜면 Swift 바인딩 생성기가 **라이브러리 의존성**으로 딸려 들어와 `.a`에 그 오브젝트가 함께 실린다(이 크레이트의 릴리스 rlib만 29,818,752바이트로 실측). 이 크레이트는 askama 템플릿을 엮어 컴파일하면서 **같은 디렉터리·같은 플래그·같은 의존성으로 다시 빌드해도 매번 다른 바이트**를 낸다(같은 조건으로 세 번 빌드해 세 개의 다른 `libuniffi_bindgen-*.rlib` 해시를 확인: `d105b327…`, `8449293c…`, `9ebfbcfd…`). 두 트리의 `.a`에서 달랐던 멤버는 이 크레이트의 오브젝트뿐이었고, 나머지는 모두 같았다. 기능을 떼어 낸 뒤 `libaether_ffi.a`에는 이 크레이트의 오브젝트가 하나도 없다(`ar t … | grep -i bindgen` → 없음).

그 기능이 필요한 것은 `uniffi-bindgen` **바이너리**뿐이므로, 라이브러리 그래프에서 떼어 냈다.

```toml
[features]
bindgen = ["uniffi/cli"]        # 바인딩 생성 바이너리 전용

[[bin]]
name = "uniffi-bindgen"
required-features = ["bindgen"]
```

`build-wallet.sh`·`build-agent.sh`만 `--features bindgen`을 준다. **`uniffi`에 `cli`를 다시 켜면 `.a` 재현성이 깨진다** — 그 도구는 지갑이 호출하지도 않는다.

### Tier 2: `scripts/repro-app-check.sh`

`.app`은 서명(Xcode가 붙이는 ad-hoc 서명 + Developer ID + 공증 티켓 스테이플) 때문에 통째로는 비트가 같아질 수 없다. 그래서 번들 안의 모든 Mach-O를 복사해 `codesign --remove-signature`로 서명을 떼고, `LC_UUID` 자리를 0으로 비운 뒤(`macho-uuid.py zero`) SHA-256을 비교하고 `LC_UUID`는 값만 함께 출력한다.

```bash
scripts/repro-app-check.sh                 # 앱을 두 번 빌드해서 비교
scripts/repro-app-check.sh A.app B.app     # 이미 빌드한 번들 두 개 비교
```

`LC_UUID`를 비교하지 않고 **출력만** 하는 이유는, 그 값이 코드가 아니라 링크 입력을 따라가기 때문이다(위 "링커의 `LC_UUID`" 참고). Xcode가 직접 링크하는 두 바이너리(`AetherWallet`, 확장)는 dSYM 조회가 UUID로 짝을 맞추므로 Xcode의 링크 결과를 그대로 둔다 — 그래서 그 둘은 "differs"로 표시된다. 번들이 안고 들어가는 Rust 바이너리(`Helpers/`)는 릴리스 스크립트가 `aether_repro_fix_uuid`로 내용 기반 UUID를 붙이므로 **비트까지 같아진다**. 즉 서명 제거 해시가 같으면 코드·데이터가 같다는 뜻이다. 빌드에는 Developer ID 인증서와 Jolt 포크가 필요하다. **서명·공증은 하지 않는다** — 서명이 붙은 산출물을 비교만 한다.

### 단위 테스트: `scripts/test-repro-scripts.sh`

cargo/Xcode 없이 빠르게 돈다. 여섯 묶음:

1. `repro-env.sh`가 에폭·`ZERO_AR_DATE`·remap을 맞게 내는지, 호출자가 준 `RUSTFLAGS`를 지키는지, 링크 플래그를 두 번 불러도 `-Wl,-reproducible`이 한 번만 붙는지, 그리고 **`-no_uuid`가 절대 들어가지 않는지**(4번 참고: 넣으면 빌드가 죽는다). 타깃 디렉터리가 있든 없든 remap 목록이 같은지도 본다(웜/콜드 빌드가 갈리면 안 된다).
2. `deterministic-zip.py`가 같은 내용을 같은 바이트로 싸는지(`.test.` 제외 포함), 내용이 바뀌면 해시가 바뀌는지.
3. `repro-app-check.sh`가 서명만 다른 두 번들, `LC_UUID`만 다른 두 번들을 "재현됨", 코드가 다른 두 번들을 "다름"(종료 코드 1)으로 판정하는지.
4. `aether_repro_fix_uuid`가 `LC_UUID`만 다른 같은 코드 두 빌드를 **비트 동일**하게 만들고, 결과가 여전히 실행되고, `dwarfdump`가 새 UUID를 읽어 주는지. 두 번 돌려도 바이트가 그대로인지(멱등), 없는 파일에는 실패하는지.
5. `macho-uuid.py`가 `dwarfdump`가 읽는 UUID와 같은 값을 읽고, `zero`가 정확히 16바이트만 건드리고, `rebuild`가 파일 내용에 따라 정해지고 멱등이며, `-no_uuid`로 링크해 UUID가 아예 없는 바이너리에는 실패하는지(조용히 넘어가지 않는다).
6. `guest-stage.sh`가 스테이지를 두 표기(`/tmp`와 물리 `/private/tmp`) 모두 remap하고, 있는 항목만 링크하며, 자기가 만들지 않은 파일은 덮어쓰지 않고, 체크아웃이 바뀌면 따라가고, `apps/prover`는 심링크가 아닌 실제 디렉터리로 두고, 스테이지 자신을 체크아웃으로 주면(populate·enter 양쪽) 링크를 자기 자신으로 돌리지 않는지. 락이 두 빌드를 직렬화하고, 중첩 빌드는 기다리지도 부모 락을 풀지도 않으며, 죽은 소유자의 락은 회수하는지.

```bash
scripts/test-repro-scripts.sh
scripts/test-reproducibility.sh   # ZIP·Mach-O 개념 검증(기존)
```

## 확장 ZIP 패키징 (`scripts/deterministic-zip.py`)

`zip` 명령은 파일 mtime·순회 순서·모드를 그대로 기록해서, 같은 파일을 두 번 싸도 두 해시가 나온다. 대신 `deterministic-zip.py`가 `SOURCE_DATE_EPOCH`(UTC) 시각, 정렬된 엔트리, `0644` 모드로 싼다. `build-extension.sh --zip`이 이 스크립트를 쓴다.

## 재현되지 않는 것 (정직하게)

- **macOS `.app` 번들 자체.** 코드서명은 개발자 개인키와 Apple TSA 타임스탬프에 의존하고, 공증 티켓을 스테이플하면 번들이 바뀐다. 그래서 번들 통짜 해시는 재현 대상이 아니다. Tier 2(서명 제거 + `LC_UUID`를 0으로 비운 해시)로만 검증한다.
- **Xcode가 직접 링크하는 두 바이너리의 `LC_UUID`.** dSYM 조회가 UUID로 짝을 맞추므로 Xcode의 링크 결과를 그대로 둔다(Tier 2 검사에서 "differs"로 표시되지만 비교 대상은 아니다). 번들에 들어가는 Rust 바이너리는 릴리스 스크립트가 내용 기반 UUID로 다시 쓰므로 이 문제가 없다.
- **맨손 `cargo build --release`의 결과.** 재현성은 스크립트가 만드는 산출물의 성질이다. `repro-env.sh`를 source 하지 않고 빌드하면 remap도 `aether_repro_fix_uuid`도 빠져서, 같은 코드라도 배포본과 다른 바이트가 나온다(경로가 새고 UUID가 링커를 따라간다). 게스트(증명 프로그램 ID)만은 맨손 빌드에서도 스테이지를 지나므로 안정적이지만, 사이드카 바이너리와 노드는 그렇지 않다. 제3자가 대조할 때는 릴리스와 **같은 스크립트**로 빌드해야 한다 — `scripts/repro-check.sh`가 그 경로를 그대로 밟는다.
- **iOS App Store 배포본.** Apple이 FairPlay DRM(`LC_ENCRYPTION_INFO`의 `cryptid=1`)으로 암호화하고, App Slicing으로 기기별로 다시 패키징하며, Apple 인증서로 재서명한다. 제3자가 App Store 바이너리를 같은 해시로 재생성하는 것은 **수학적으로 불가능**하다([Wallet Scrutiny 방법론](https://walletscrutiny.com/methodology/)). 대신 배포 전 **미서명 IPA/xcarchive**의 SHA-256과 `LC_UUID`를 릴리스 노트에 공개해 대조한다.
- **툴체인·SDK 버전.** 같은 Xcode/rustc/zlib이어야 같은 바이트가 나온다. `rust-toolchain.toml`은 Rust를 고정하지만 Xcode 버전은 아직 고정하지 않는다(Tier 2 검증에는 링커 버전이 영향을 준다). `-Wl,-reproducible`은 이 저장소가 확인한 Xcode 26.6(ld-1267)에서 동작한다 — 더 오래된 링커는 이 플래그를 모를 수 있다.
- **제3자 크레이트의 컴파일 자체가 비결정적일 수 있다.** 위의 `uniffi_bindgen`이 그런 예다(같은 입력·같은 플래그로 매번 다른 바이트). 이 저장소가 고칠 수 없는 코드지만 재현성은 저장소 전체의 성질이므로, 의존성을 추가하거나 기능을 켤 때마다 `repro-check.sh`로 확인한다 — 특히 **도구 상자 크레이트가 라이브러리 그래프로 딸려 들어오지 않는지**를 본다.
- **Jolt 포크의 위치와 리비전.** `apps/prover`는 Jolt·Akita를 `/Volumes/workspace/aether-jolt` 아래에서 **절대 경로**로 링크하고(`Cargo.toml`의 `[patch."https://github.com/…"]` 항목 11개 포함), 그 경로는 Aether 체크아웃의 경로와 똑같이 `-C metadata` 해시에 들어간다. 즉 포크가 다른 경로에 있으면 **증명 프로그램 ID가 달라진다** — remap은 이 해시에 닿지 않는다. 이 저장소가 고정할 수 있는 것은 자기 체크아웃 경로뿐이므로(`scripts/guest-stage.sh`), 포크는 **릴리스 문서에 적은 경로에 두고** 릴리스마다 포크 커밋을 함께 기록한다(포크 리비전이 다르면 ID가 달라지는 것은 당연하다).

## 릴리스 절차

1. `scripts/test-repro-scripts.sh` (빠른 확인).
2. `scripts/repro-check.sh all` — 다섯 항목(노드·FFI·프로그램 ID·사이드카·확장)이 두 디렉터리에서 같은 바이트로 나오는지. 통과한 해시를 릴리스 노트에 적고, 증명 프로그램 ID와 함께 Jolt 포크의 **경로와 커밋**(위 "Jolt 포크의 위치와 리비전")을 적는다.
3. `scripts/repro-app-check.sh` — 앱을 두 번 빌드해 서명 제거 + `LC_UUID` 0으로 비운 Mach-O 해시 대조(`LC_UUID`는 값만 함께 출력).
4. iOS는 미서명 아카이브 해시 + `LC_UUID`만 공개하고 "App Store 바이너리는 검증 불가"를 명시한다.
