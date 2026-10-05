# 빠른 WebAssembly 소식 조사 (2026-10-05)

## 결론 먼저

창업자가 들은 "아주 빠른 wasm"은 Frank Denis(libsodium 저자)의 벤치마크 글 **"Performance of WebAssembly runtimes in 2026"**(2026-06-23 게시)일 가능성이 가장 높습니다. 이 글은 9월 9일 무렵 HN에서 102점을 받았고, 재인용 매체는 "One New Instruction Closes the Gap to Native"라는 제목을 달았습니다. 여기서 말하는 새 명령어는 **wide-arithmetic**입니다(`i64.mul_wide_u`, `i64.add128` 등 128-bit 연산).

**우리 브라우저 검증에는 당장 효과가 없습니다.** 이 명령어를 정식으로 켠 브라우저가 아직 없고, 우리가 쓰는 blst는 wasm32에서 이 명령어가 쓰일 수 없는 방식으로 컴파일되기 때문입니다.

## 1. 후보와 검증 여부

| 후보 | 출처·날짜 | 수치 | 판정 |
|---|---|---|---|
| **wide-arithmetic**(Denis 벤치) | 00f.net 2026-06-23 | Wasmer 7.1.0: 2.08x → **1.33x** native, Wasmtime 46: 2.41x → **1.46x**. Node 26.3.1은 7.95x | **검증된 수치.** 단, 서버 런타임 기준이고 브라우저는 측정하지 않음 |
| wasm→C(wasm2c) | 00f.net 2026-07-08 | wide-arith를 쓰면 Wasmer 대비 0.887x 시간 | 검증됨. 서버 AOT용이라 우리와 무관 |
| Wasmer 7.0 | 2026-01-30 | "95% native"(Coremark) | 벤더 주장. 독립 측정은 1.33x |
| Firefox 155 wide-arith 기본 활성화 | Mozilla dev-platform, 2026-08-12 intent | 수치 없음 | 진행 중 |
| "Wasm 3.0이 2026-06-13에 나왔다" | byteiota | — | **오류.** 실제 발표는 2025-09 |
| WebGPU BLS12-381(voidash) | GitHub | CPU 쪽 pairing 1.44 ms, GPU 쪽은 미구현 | **과장.** 브라우저가 아닌 native wgpu이고 미완성 |

webassembly.org 기능표(features.json) 기준 브라우저 지원 현황:

- **wide-arithmetic:** Chrome은 `--js-flags` 플래그가 있어야 하고, Firefox는 Nightly에서만 켜져 있으며, Safari는 JSC 플래그가 필요합니다. **정식으로 켠 브라우저는 없습니다.**
- **SIMD128:** 모두 지원(Chrome 91, Firefox 89, Safari 16.4).
- **threads:** 모두 지원(Chrome 74, Firefox 79, Safari 14.1).
- **relaxed SIMD:** Chrome 114, Firefox 145. Safari는 플래그 필요.
- **memory64:** Chrome 133, Firefox 134. Safari는 Technical Preview 단계.
- **JSPI:** Chrome 137, Firefox 153, Safari 27.

## 2. 실측: 현재 우리 경로 (M1 Max, Node 22 / V8)

`blst 0.3.16`을 그대로 wasm32로 빌드해서 쟀습니다. 측정용 크레이트는 scratchpad에 만들었고 repo는 건드리지 않았습니다.

| 구성 | verify 1회 (hash-to-curve + pairing check) |
|---|---|
| wasm, MinSig(우리 light client 방식) | **11.7 ms** |
| wasm, MinPk | 12.9–15.5 ms |
| wasm + `-C target-feature=+simd128` | 12.9 ms (**변화 없음**) |
| native (aarch64 asm) | 0.68 ms |

브라우저 wasm은 native보다 약 **17배** 느립니다. Denis 벤치의 Node 7.95x보다 격차가 큰 이유는 blst 소스 `blst/src/vect.h`에 있습니다. wasm32에서는 `LIMB_T_BITS 32`, 즉 32-bit limb와 C 경로로 내려가는데, native는 64-bit 곱셈을 쓰는 손으로 짠 asm을 씁니다.

이 수치는 blst만 떼어 낸 측정입니다. 실제 블록 데이터와 `verify_finalized`로 돌린 것이 아니고, Safari와 Firefox에서는 재지 않았습니다.

블록 하나에 pairing을 한 번 하는 지금 구조에서 확장앱은 블록당 약 10–15 ms를 씁니다. 모바일이나 저사양 기기에서는 2–4배로 추정합니다. 확장앱에는 체감할 문제가 아닙니다. 다만 explorer가 블록 1,000개를 연속 검증하면 약 12초가 걸리므로 그때는 의미가 생깁니다.

## 3. 무엇을 바꾸면 얼마나 빨라지나 (추정)

- **simd128:** 효과 없음(실측). 큰 정수 곱셈은 SIMD로 벡터화되지 않습니다. 게다가 RUSTFLAGS는 blst의 C 코드에는 적용되지도 않습니다.
- **wide-arithmetic:** blst를 패치해서 wasm32에서도 64-bit limb와 `__int128`을 쓰게 하고 clang에 `-mwide-arithmetic`을 주면, Denis의 수치로 보아 1.5–2.5배가 기대됩니다. 하지만 정식 지원 브라우저가 없어서 기능 감지와 이중 빌드가 필요하고, 암호 라이브러리를 직접 패치하는 위험도 있습니다. **지금은 하지 않는 편이 맞습니다.** Chrome과 Safari가 정식 지원하면 다시 보면 됩니다.
- **배치 검증:** blst의 `verify_multiple_aggregate_signatures` 방식(random linear combination)을 쓰면 블록 N개에서 final exponentiation을 한 번으로 줄일 수 있습니다. 대량 동기화에서 약 2배가 예상됩니다. 순수 코드 변경이라 브라우저 지원 문제가 없지만, commonware API가 이를 지원하는지는 확인해야 합니다.
- **threads / Web Worker:** pairing 한 번은 거의 병렬화되지 않습니다. 여러 블록을 동시에 검증할 때만 코어 수만큼 빨라집니다.
  - SharedArrayBuffer 없이 Worker 여러 개에 블록을 나눠 맡기는 방식이 가장 단순합니다.
  - SAB 기반 wasm threads를 쓰려면 nightly `build-std`와 `+atomics,+bulk-memory`가 필요하고, explorer는 COOP/COEP 헤더를 내야 합니다.
- **WebGPU:** pairing 한 번에는 부적합합니다. dispatch와 readback 왕복이 ms 단위입니다. MSM이나 수천 건 일괄 처리에서나 의미가 있고, 성숙한 BLS12-381 WGSL 구현도 없습니다.
- **wasm-opt:** wasm-pack이 기본으로 실행합니다. 속도 이득은 보통 0–10%입니다.

## 4. Chrome 확장(MV3) 제약

- 현재 CSP의 `'wasm-unsafe-eval'` 설정은 올바릅니다.
- MV3 service worker는 `new Worker`를 만들 수 없습니다. 병렬 처리가 필요하면 `chrome.offscreen` document에서 Worker를 띄워야 합니다.
- SAB를 쓰려면 manifest에 `cross_origin_embedder_policy: require-corp`와 `cross_origin_opener_policy: same-origin`을 넣어야 하고, 이 조건은 확장 페이지에만 적용됩니다.
- 확장앱은 블록을 하나씩 검증하므로 이런 설정은 **필요 없습니다.**

## 5. 노력과 위험

| 옵션 | 노력 | 위험 | 이득 |
|---|---|---|---|
| 아무것도 안 함 | 0 | 0 | 확장앱에는 현재 충분 |
| 배치 검증(explorer) | 2–3일 | 낮음 | 대량 동기화 약 2배 |
| Worker 병렬화(explorer) | 2–4일 | 낮음 | 코어 수만큼 |
| wide-arith + blst 패치 | 1–2주 | **높음**(암호 코드 포크, 플래그 뒤에 숨은 기능) | 1.5–2.5배, 미래 |
| WebGPU pairing | 수 주 이상 | 높음 | 우리 규모에서는 없음 |

## 권고

이번 소식은 서버 런타임 이야기라 우리 브라우저 검증에 바로 적용할 것은 없습니다. 지금 확장앱의 블록당 약 12 ms는 충분히 빠릅니다.

**가장 작은 다음 단계:** `verify_finalized`를 실제 블록 데이터로 돌리는 wasm 벤치를 `crates/wasm`에 추가하고, Chrome·Safari·Firefox에서 한 번씩 측정합니다(반나절). 그 숫자로 explorer 동기화 시간이 허용 범위를 넘는지 판단합니다.

넘는다면 **배치 검증 → Worker 병렬화** 순으로 진행하고, wide-arithmetic은 Chrome과 Safari가 정식 지원한 뒤에 재검토하는 것을 권합니다.

벤치용 크레이트는 `/private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73-7af2-437d-8a06-ab75e6c24a76/scratchpad/blsbench`에 있고, repo 파일은 수정하지 않았습니다.

## Sources

- [Performance of WebAssembly runtimes in 2026 (Frank Denis)](https://00f.net/2026/06/23/webassembly-runtimes-2026/)
- [The best WebAssembly runtime may still be no runtime at all](https://00f.net/2026/07/08/webassembly-compilation-to-c-2026/)
- [HN discussion](https://news.ycombinator.com/item?id=49623933) / [zeli.app summary](https://zeli.app/story/49623933)
- [Mozilla: Intent to ship Wide Arithmetic (Firefox 155)](http://www.mail-archive.com/dev-platform@mozilla.org/msg01862.html)
- [wide-arithmetic proposal](https://github.com/WebAssembly/wide-arithmetic/blob/main/proposals/wide-arithmetic/Overview.md)
- [webassembly.org feature status (features.json)](https://webassembly.org/features/)
- [Wasmer 7.0 (Phoronix)](https://www.phoronix.com/news/Wasmer-7.0-Released) / [wasmer.io/posts/wasmer-7](https://wasmer.io/posts/wasmer-7)
- [voidash/webgpu-examples (WGSL BLS12-381)](https://github.com/voidash/webgpu-examples)
- [Chrome extensions: cross-origin isolation](https://developer.chrome.com/docs/extensions/mv3/cross-origin-isolation)
- [Offscreen Documents in MV3](https://developer.chrome.com/blog/Offscreen-Documents-in-Manifest-v3)
- [byteiota Wasm 3.0 article (date flagged as wrong)](https://byteiota.com/webassembly-30-spec-release/)
