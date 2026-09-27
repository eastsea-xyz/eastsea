# aether-prover

블록 증명 사이드카. 노드(`aether run`)가 서브프로세스로 띄운다. Jolt 스파이크
(`spike/jolt-block`)의 `prove_block` 게스트를 그대로 옮겼고(P-256·BLAKE3 inline 포함),
게스트 ELF를 **빌드 시점에 미리 컴파일해 바이너리에 내장**한다. 실행 시 cargo, jolt CLI,
riscv 툴체인, jolt/akita 체크아웃 어느 것도 필요 없다.

## 빌드

필요: `/Volumes/workspace/aether-jolt/jolt`(브랜치 `aether`)와 옆의 `akita` 체크아웃,
`jolt` CLI, rustup (`export PATH=$HOME/.cargo/bin:$PATH`).

    cd apps/prover
    cargo build --release                                   # Metal (기본, Apple Silicon)
    cargo build --release --no-default-features --features akita --target-dir target-akita   # CPU

`build.rs`가 `build-guest.sh`로 게스트 ELF를 만들어 `include_bytes!` 한다. 게스트나
`crates/` 가 바뀌면 다시 빌드된다. 미리 만든 ELF를 쓰려면
`AETHER_PROVER_GUEST_ELF=/path/prove_block.elf cargo build --release`.

## 사용

    aether-prover prove <input.postcard> <proof.out>
      → {"commitment":"<hex>","proof_bytes":N,"seconds":S}      실패 시 exit 1
    aether-prover verify <proof> <claim-hex>
      → exit 0: 증명이 내장 ELF에 대해 검증되고, 게스트가 panic 하지 않았고, 출력 = claim
    aether-prover serve          # stdin 한 줄 = JSON 요청, stdout 한 줄 = JSON 응답
      {"cmd":"prove","input":"a.postcard","out":"a.proof"}
      {"cmd":"verify","proof":"a.proof","commitment":"<hex>"}
      {"cmd":"info"}
    aether-prover self-test [n]  # n-tx 샘플 블록 증명 + 검증 + 오답/변조/절단 증명 거부
    aether-prover sample-input <n> <out.postcard>
    aether-prover info           # 내장 ELF SHA-256 (= 증명 대상 프로그램 ID)

입력은 postcard `aether_proving::block::BlockInput`. 게스트 출력은
`claim(C, prover)`(`aether_proving::block::claim`): 블록 명제의 commitment C를 입력의
증명자 주소와 묶은 32바이트다. 그래서 증명을 본 다른 사람이 자기 주소로 청구할 수 없다.
JSON 필드와 인자 이름은 역사적 이유로 `commitment`이지만 값은 이 claim이다. 입력은
게스트에 **untrusted advice**로 들어가므로 검증자는 입력 없이 claim 만으로 검증한다(명제:
"어떤 입력에 대해 이 프로그램이 panic 없이 claim(C, prover)를 출력했다"). 증명 범위는
13-protocol-2.md §1: 블록의 시스템 쓰기 뒤 상태에서 트랜잭션을 실행한 결과. `prove`는 먼저 네이티브 실행으로 입력을 검사하고,
증명 후 게스트 출력과 네이티브 결과를 대조한다. 진단은 stderr, stdout은 JSON 한 줄.

## 측정 (M1 Max, Metal, 2026-09-27, 머신 부하 load avg 25~75 상태)

| | 10 tx (2^24) | 50 tx (2^26) |
|---|---|---|
| 증명, 콜드 프로세스 | 28.6~31.7 s | 71.0~81.2 s |
| 증명, `serve` 두 번째부터 | – | 47.2 s |
| 증명 크기 | 97.9 kB | 98.0 kB |
| 검증, 콜드 프로세스 (`verify`) | 1.3~1.5 s, RSS 0.79 GB | 1.26~1.32 s, RSS 1.07 GB |
| (verifier 전용 셋업 이전) | 1.6~2.8 s, RSS 1.47 GB | 26.1~26.7 s, RSS 16.0 GB |
| 검증, `serve`/웜 | 0.23~0.26 s | 0.33 s |
| 프로그램 전처리 (ELF→바이트코드) | 0.7 s | 0.7 s |

바이너리 60 MB (strip 시 44 MB), 내장 ELF 3.6 MB. 같은 입력 → 같은 증명 바이트(결정적).

재현:

    ./target/release/aether-prover self-test 10
    ./target/release/aether-prover sample-input 50 /tmp/b50.postcard
    ./target/release/aether-prover prove /tmp/b50.postcard /tmp/b50.proof
    ./target/release/aether-prover verify /tmp/b50.proof <commitment>

## 알아둘 점

- **콜드 검증은 verifier 전용 셋업으로 한다.** 검증 키는 prover 셋업(2^26 에서 공개
  행렬 2^28 원소 = 4 GiB)을 만들지 않고 검증이 읽는 공개 행렬 접두부(2^26 에서 2^24
  원소 = 256 MiB)만 시드에서 유도한다. setup-prefix 커밋(접두부 전체를 커밋해야 해서
  검증자가 직접 만들면 prover 셋업 비용이 든다)은 jolt 포크에 12개·4.6 kB 로 미리
  유도해 내장했다(`gen_akita_setup_prefixes`, 2^16~2^26 전 모양 커버, 테스트가 재유도해
  대조). 콜드 RSS 1.07 GB 중 0.41 GB 는 프로그램 전처리(바이트코드 2^21)다.
- 증명이 선언한 `ram_K`는 메모리 레이아웃 상한으로 검사한 뒤 셋업을 만든다(DoS 방지).
- ELF가 곧 프로그램 ID다: prover 와 verifier 는 같은 ELF 를 내장해야 한다.
  `build-guest.sh`는 `--remap-path-prefix`로 경로를 지워 체크아웃·툴체인 위치와 무관한
  ELF를 만든다. 릴리스에서는 한 번 빌드한 ELF를 `AETHER_PROVER_GUEST_ELF`로 고정하고
  `info`의 SHA-256 을 기록할 것.
- `scripts/prover-program.sh`가 이 사이드카를 빌드하고 SHA-256(프로그램 ID)을 출력한다.
  `build-wallet.sh`·`testnet-reset.sh`는 이 값을 `AETHER_PROVER_PROGRAM`으로 넣어 노드를
  빌드하고, 노드는 시작할 때 사이드카의 `info`와 대조한다. Jolt 포크가 없으면 macOS 앱
  빌드는 실패한다.
- Akita 스케줄 카탈로그(`*.aks`)는 jolt 포크의 `embedded-schedules` 기능으로 바이너리에
  들어간다(없으면 jolt 체크아웃 경로에서 런타임에 읽는다).
