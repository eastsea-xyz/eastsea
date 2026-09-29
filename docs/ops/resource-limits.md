# 리소스 제한: 맥이 폭주하지 않게 하는 예산

## 배경 (2026-09-29 사고)

64 GB 맥에서 `aether-prover`가 14 GB 메모리·350% CPU를 쓰면서 스왑 17.5/18.4 GB(95%), 로드 평균 ~1000까지 갔다. 같은 맥에서 돌던 테스트넷 검증자 4개는 ~0.45 블록/s로 느려졌다. macOS는 프로세스별 RSS 한도를 강제하지 않는다(`RLIMIT_RSS`·`RLIMIT_AS` 무시) — 그래서 노드가 직접 지켜본다.

## 플래그

`aether run`·`aether node`·`aether follow` 모두 같은 플래그를 받는다(지갑 앱의 노드는 `aether run`으로 전달).

| 플래그 | 기본값 | 뜻 |
|---|---|---|
| `--prover-max-memory` | `auto` | 증명 사이드카의 물리 풋프린트 상한. `auto` = RAM의 25% (최소 4 GB). `0`이면 증명을 아예 하지 않는다. |
| `--prover-threads` | 코어의 절반 | 사이드카의 rayon 워커 수(`RAYON_NUM_THREADS`). 절반보다 올릴 때만 쓴다. |
| `--prover-on-battery` | 끔 | 배터리에서도 증명을 허용한다. 끔이면 배터리에서 증명이 일시 정지된다. |
| `--max-memory` | `auto` | 노드 자기 자신의 히스토리 캐시 예산. `auto` = RAM의 25% (최소 2 GB). |
| `--min-free-disk` | 5 GB | 데이터 볼륨의 최소 여유. 이보다 낮으면 새 era 파일·샤드를 쓰지 않고 증명도 일시 정지. `0`이면 가드 끔. |

크기 표기: 숫자만 쓰면 GB("8"), 단위를 붙이면 정확히("512M", "6G", "8589934592B"). 지갑 설정(설정 ▸ 리소스)이 같은 플래그를 만든다(`ProverFlags.build`).

## 증명 사이드카 와치독 (메모리 상한)

- 2초마다 사이드카의 물리 풋프린트를 읽는다(`proc_pid_rusage` RUSAGE_INFO_V4의 `ri_phys_footprint` — 활성 모니터의 "메모리" 칸과 같은 값).
- 상한을 넘으면 **pid로 증명 프로세스를 죽인다**(요청이 사이드카의 io 잠금을 최대 30분 잡고 있어도 기다리지 않는다), 이유와 숫자를 한 번 경고 로그로 남기고, 백오프 후 재시작한다.
- 백오프는 증명 하나가 성공할 때마다 초기화된다: 60초부터 매번 두 배, 최대 30분(60 → 120 → 240 → 480 → 960 → 1800 → 1800…).
- 상태는 `aether_proverStatus`에 `paused: "memory"`, `memory_bytes`(마지막 샘플), `memory_cap`, `threads`로 나온다.

## 시스템 우선 (압력·스왑·배터리)

- 메모리 압력이 warn 이상(`kern.memorystatus_vm_pressure_level`, 커널이 답하지 않는 기기에서는 free 페이지 + 커널이 되찾을 수 있는 페이지가 RAM의 10% 미만이면 warn·5% 미만이면 critical)이거나 스왑 사용이 절반을 넘으면 증명이 **새 작업을 받지 않는다**. critical이면 돌고 있던 증명도 죽인다(백오프 없이 — 모니터의 일시 정지가 재시작을 막는다).
- 시스템이 정상으로 돌아온 뒤 **5분**이 지나야 증명이 재개된다(경미한 출렁임에 재개/정지를 반복하지 않게).
- `--prover-on-battery`가 없으면 배터리에서 증명이 일시 정지된다(노드 자체는 `Only while on the power adapter` 설정이 따로 있다).
- 이유는 `paused`에 드러난다: `memory`(사이드카 상한), `pressure`, `battery`, `disk`.

## CPU

- 증명 사이드카는 낮은 스케줄러 우선순위(`setpriority(PRIO_PROCESS, 15)`)로 뜬다. CPU 경합에서 밀리는 것뿐, 디스크나 QoS에는 영향이 없다.
- 워커 스레드는 기본적으로 코어의 절반(`RAYON_NUM_THREADS`). 검증 사이드카(합의용, 1초 안에 답해야 한다)와 노드 프로세스 자신은 보통 우선순위를 유지한다.

## 노드 자기 캐시 예산 (`--max-memory`)

히스토리가 쌓이며 함께 자라는 **메모리 내** 캐시만 이 예산 안에 든다:

- 블록 요약 `blocks`(BTreeMap<높이, BlockSummary>) — 예산이 빠듯하면 가장 오래된 **봉인된 era**부터 내보낸다. era 파일이 디스크에 있을 때만 내보내고, 열려 있는 era는 절대 두지 않는다.
- 영수증 `receipts`(해시 → (높이, Receipt)) — 같은 era 단위로 함께 간다.
- 나머지(`executed` 최근 실행, `recent` 32개, 멤풀 64 MB, era 캐시 1개)는 이미 상한이 있어 그대로다.

내보낸 높이도 서비스는 계속된다: `aether_getBlock`·`history_proof`는 era 파일에서 읽는다(`pruned_below`와 `cache_below` 중 큰 값 아래는 전부 파일로). **era 파일이 없는 네트워크(7780 규칙)는 아무것도 내보내지 않는다** — 파일이 없으면 그 높이를 받아줄 곳이 없다.

노드는 10분마다 자기 풋프린트를 로그에 남긴다(`this node's memory (physical footprint, every 10 min)`).

## 디스크 가드 (`--min-free-disk`)

- 데이터 볼륨의 여유(`statfs` `f_bavail`)가 최소값 아래로 내려가면: **새 era 파일 봉인과 샤드 쓰기를 멈추고**(스토어의 다른 쓰기는 그대로) 증명을 일시 정지하고, 경고를 로그에 남긴다.
- 블록은 스토어에 staged로 남는다 — 다음 시작 때 `seal_pending`이 따라잡고, `prune::cutoff`는 봉인 안 된 era를 잘라내지 않으므로 아무것도 잃지 않는다. 샤드 정리(공간을 ** freeing**)는 계속 허용한다.
- 최소값 + 2 GB 이상 올라와야 재개한다(경계에서 깜빡이지 않게 하는 히스테리시스).
- `aether_status`의 `resources` 객체: `disk_free`, `disk_low`, `min_free_disk`, `proving_paused`. 지갑은 `disk_low`를 보고 "디스크 공간 부족"을 보여 준다.

스토어 오류 복구(자가 치유)는 이 문서의 범위가 아니다 — 별도 작업.

## 확인 방법

```sh
curl -s localhost:18545 -X POST -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"aether_status","params":[]}' | jq .result.resources
```

`aether_proverStatus`는 `running`, `proving`, `paused`, `memory_bytes`, `memory_cap`, `threads`, `proofs` 등을 준다.

## 테스트

- `cargo test -p aether-node --lib -- resources:: prover::` — 크기/스왑 파싱, 플래그 해석, 디스크 히스테리시스, 5분 재개, critical 구분, 페이지 폴백, 기계 읽기; 백오프 사다리; 가짜 사이드카(핸드셰이크를 말하고 awk로 256 MB를 잡는 셸 스크립트)를 상한 넘게 키워 와치독이 죽이고 백오프하는 것.
- `cargo test -p aether-node --bin aether resource_` — `node`·`follow`·`run`의 플래그 파싱과 `run`의 자식 전달.
- `cargo test -p aether-node --test resources` — 캐시 예산으로 era 단위 퇴거, 퇴거된 블록·증명이 era 파일로 그대로 서비스되는 것, era 없는 네트워크는 퇴거하지 않는 것.
- `cargo build -p aether-node`는 `AETHER_PROVER_PROGRAM=$(scripts/prover-program.sh)`와 함께. 앱은 `xcodegen generate` 뒤 `xcodebuild`(AetherWallet / AetherWalletIOS 스킴).
- `swiftc -o /tmp/resources-check apps/wallet/Sources/ProverFlags.swift apps/wallet/Tests/resources/main.swift && /tmp/resources-check` — 지갑이 만드는 플래그 목록.
