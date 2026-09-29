# aether-node 메인넷 준비도 갭 분석 (PDCA Check, 2026-09-29)

기준: `/Volumes/workspace/aether-node` 체크아웃(phase1-nextgen HEAD). 요청대로 진행 중인 작업 7건(인계 명단 바인딩·예비 키 강화, 09-29 레드팀 수정, 결제 요청·승인 철회, 릴리스 로그·빌더 공동서명, WebTransport 확장, 푸시 구독, 그룹 대비 제네시스 필드)은 미구현으로 셌습니다.

**결론: 메인넷 조건은 아직 하나도 완전히 충족되지 않았습니다.** 가장 큰 문제는 두 가지 코드 결함입니다.
- **G1:** 새 제네시스가 프로토콜 1로 시작합니다. 그래서 증명 시장, 에포크당 등록 상한, 16석 증가 규칙이 모두 꺼진 채 열립니다.
- **G2:** 앱과 확장 지갑이 팁을 항상 1 gwei로 넣습니다. 그래서 잔액 0인 계정은 거래할 수 없고, 사전 발행과 faucet이 없는 메인넷에서는 새 Mac이 앱으로 등록조차 못 합니다.

---

## 1. Match Rate: **56%** (⚠️, 70% 미만: 설계와 구현을 맞춰야 함)

**계산 방법:** 메인넷과 관련된 설계 항목 55개를 확인했습니다. 구현은 1점, 부분 구현은 0.5점, 미구현은 0점입니다.
- 구현 25, 부분 12, 미구현 18 → (25 + 6) / 55 = **56.4%**
- 제외한 것: 다른 저장소에 있어 여기서 확인할 수 없는 DEX 화면·토큰 팩토리·런치패드(`apps/explorer/token-sources.json:2`에 "aether-dex, aether-launchpad-demo에서 복사"라고 적혀 있음), 그리고 🧑 항목.

**12-launch-plan.md:106 "빠른 메인넷" 조건 (1)~(7) 기준으로 보면: 충족 0/7, 부분 2 ((1), (7)), 나머지는 시간·🧑·감사 대기.**

| # | 설계 항목 (출처) | 상태 | 근거 |
|---|---|---|---|
| 1 | 프로토콜 3 검증자 증가, 16석 (13:30) | ✅ | `crates/node/src/rotation.rs:153,160` |
| 2 | 시간대 분산 추첨·조기 교체 (13:98-110) | ✅ | `rotation.rs:388,447`, `chain.rs:1902,2205` |
| 3 | 예비 키 규칙·봉사 몫·생존 규칙 (15:256-271) | ✅ | `crates/rewards/src/lib.rs:387-510`, `rotation.rs:525,554` |
| 4 | `reserve-keys.sh` 운영 | ✅ | `scripts/reserve-keys.sh` |
| 5 | FOCIL Inline, DurableVote, 제안 정렬, nofile 한도 | ✅ | consensus-recovery.md, `main.rs` `raise_nofile_limit` |
| 6 | `AETHER_RECOVER_CONSENSUS` | ✅ | consensus-recovery.md |
| 7 | 멈춤 알림·잠자기 방지 (13:90) | ✅ | `apps/wallet/Sources/NodeController.swift:401-405,420` |
| 8 | 메인넷이 제네시스부터 프로토콜 2·3 규칙 (15:90 "업그레이드 불필요") | ❌ | 아래 G1 |
| 9 | 인계 명단 바인딩·예비 키 강화 | ❌ | 진행 중 |
| 10 | 레드팀 09-29 노드·네트워크 수정 | ❌ | 진행 중 |
| 11 | 그룹 대비 제네시스 필드 (13:135) | ❌ | `roster.rs:40-79`에 group·위원회 크기 필드 없음 |
| 12 | 시뮬레이션: 크래시 재시작·비잔틴 주입 (12:16) | ✅ | `crates/node/tests/sim.rs`: 저장소 체크포인트 재시작(서명 인계 대기 중 포함), 침묵 생산자, 이중 서명 투표 차단, 시계 오차를 검증함 |
| 13 | 퍼징(cargo-fuzz)·Quint (12:17) | 🔶 | `fuzz/`에 파서 타깃 5개와 `scripts/fuzz-smoke.sh`가 있음. Quint 명세와 BAL·증명 타깃은 아직 없음 |
| 14 | F-P0 "위원 1/3 이상 영구 소실 시 재시작 절차" (13:95-96) | 🔶 | mainnet-launch.md §8에 "새 제네시스" 한 줄만 있음 |
| 15 | 매끄러운 감쇠 발행과 바닥 (15:305-316) | ✅ | `rewards/src/lib.rs:73-133` |
| 16 | 노드 몫 1/16·워밍업·중립일 | ✅ | `rewards/src/lib.rs:227-337` |
| 17 | 증명 몫: 등록 운영자만, 1/16 상한 | ✅ | `chain.rs:2339-2344` |
| 18 | 비콘 4슬롯, 수수료 없는 payload | ✅ | `rewards/src/beacons.rs`, `node/src/beacons.rs` |
| 19 | 하루 한 번 재확인과 속도 제한 | ✅ | `node/src/devicecheck.rs` |
| 20 | 앱의 DeviceCheck 토큰 파일 기록 (15:299 "남은 일") | ✅ | `NodeController.swift:199-217` (문서가 낡음) |
| 21 | `aether_rewardStatus` | ✅ | `node/src/rewards_view.rs` |
| 22 | 앱 보상 카드, 활동 항목, **첫 보상 알림** (15:66-73) | 🔶 | 카드는 있음(`Earnings.swift:712-876`). 첫 보상 알림은 없음(`LocalNotice.post` 호출은 멈춤 알림 2개뿐) |
| 23 | 세무 CSV (13 C3) | ✅ | `NodeController.swift:314-328` |
| 24 | 공개 네트워크 화면(N, 분배율) (15:73) | ❌ | `apps/explorer/js`에 rewardStatus 없음 |
| 25 | 시간대 프로필 (15:225) | ✅ | `rewards/src/beacons.rs` |
| 26 | 운영자 색인(`is_operator`가 후보 전체를 훑음) (15:301) | ❌ | `rewards/src/lib.rs:337` |
| 27 | 재현 가능한 증명 프로그램 ID (15:161 "메인넷 고정 전") | ❌ | `scripts/prover-program.sh`에 remap 없음 |
| 28-29 | B1 측정, B2 history_root | ✅ | 13:41-42 |
| 30 | B3 에라 파일 (묶음 서명 제외) | 🔶 | `node/src/era.rs:34` "Not yet: aggregated finality signature" |
| 31-32 | B4 가지치기, B5 조각 1단계 | ✅ | `prune.rs`, `shards.rs`, `era_net.rs` |
| 33 | 목표치 아래 기본 수수료 0 | ✅ | `execution/src/fees.rs:22-27,64-70` |
| 34 | **지갑이 잔액 0이면 팁 0** (12:106, 13 E1) | ❌ | 아래 G2 |
| 35 | 레지스트라 키를 서명 전용 Mac의 Secure Enclave에 (12:115) | ❌ | `main.rs:329,725,1417`: `<data>/registrar.key` 파일 |
| 36 | 위원회가 레지스트라 키 교체·철회 (12:116) | 🔶 | 교체는 `upgrade.rs:38`, `chain.rs:929-932`. 레지스트라를 없애는 철회 경로는 없음 |
| 37 | 에포크당 신규 등록 상한 온체인 (12:115-116) | 🔶 | `execution/src/registry.rs:61-70` `MAX_PER_EPOCH=16`. **프로토콜 2 업그레이드 때만** 설치됨 |
| 38 | 업그레이드 7일 뒤 발효, 앱이 미리 알림 (12:115) | ❌ | `chain.rs:1467-1475`: 최소 예고가 **1 에포크(1시간)**. 앱은 업데이트 확인만 함(`NodeController.swift:331-340`) |
| 39 | 재현 가능한 빌드, 업데이트 서명 지문 표시 (12:115) | ❌ | remap은 `build-extension.sh:12`에만 있음. 앱 UI에 지문 표시 없음 |
| 40 | 릴리스 로그·빌더 공동서명 | ❌ | 진행 중 |
| 45 | 토큰 잠금·베스팅 (17) | 🔶 | 컨트랙트는 `contracts/src/TokenLocker.sol`. 지갑·런치패드 화면 연결은 없음(17:80) |
| 46 | 이름 서비스 .aeth (12:70 "메인넷 전 만들 것") | ❌ | `contracts/src`에 없음 |
| 47 | 금고 (16) | 🔶 | 컨트랙트는 `AetherVault.sol`. 앱·확장 UI 없음, 배포 안 함(16:89-91) |
| 48 | 결제 요청·인보이스 (E16) | 🔶 | `aether://pay`만 있음(09:91). 요청·철회는 진행 중 |
| 49 | 블록 탐색기 (E4, 브라우저 인증서 검증) | 🔶 | `apps/explorer/README.md:45,55`: "Nothing … is verified in the browser" |
| 50 | 직접 호스팅 화면의 제재 명단 확인·지역 차단 (12:70,114) | ❌ | `apps/` 전체에 OFAC·geo 코드 없음 |
| 51 | 머클 청구·일괄 전송 (17-2) | ✅ | `contracts/src/MerkleDistributor.sol` |
| 52 | 지갑 읽기 분산 (08:71-81) | ✅ | ffi `Spread`, net `WalletServers` |
| 53 | 로컬 노드 없는 확장 (WebTransport) | ❌ | 진행 중 |
| 54 | `eth_subscribe` 구독 (13 F6) | ❌ | `crates/node/src`에 없음 |
| 55 | k-of-n 가디언 복구와 `addOwner`의 FFI·앱 노출 (12:19) | ❌ | `crates/ffi/src/lib.rs:1560-1563`: threshold가 1보다 크면 거부 |
| 56 | Pkarr 프로젝트 서명 부트노드 목록 (08:30-34) | ❌ | `crates/net/src/lib.rs:6`: 노드별 pkarr 레코드만 있음 |
| 57 | 메인넷 리허설 스크립트 | ✅ | `scripts/mainnet-rehearsal.sh` |
| 58 | 소스 공개 준비 (스캔, MIT/Apache 라이선스) | 🔶 | `scripts/prepublish.sh`(gitleaks)는 있음. 루트에 `LICENSE-MIT`·`LICENSE-APACHE` 파일이 없음(README:161, Cargo.toml:8에만 명시) |
| 59 | README 규칙 문장 (15:72) | ✅ | `README.md:101-105` |

---

## 2. 메인넷을 막는 갭

| ID | 설계 항목 | 근거 | 심각도 |
|---|---|---|---|
| **G1** | 메인넷은 제네시스부터 노드 보상·증명 시장·16석 증가 규칙 (15:90, 13 E1) | `ChainConfig`(`chain.rs:40-70`)와 `NetworkFile`(`roster.rs:40-79`)에 프로토콜 필드가 없음. 일정표가 비면 `protocol_at`이 1을 돌려줌(`upgrade.rs:45-47`). 그 결과 증명 거부(`chain.rs:925-926`), 증가 추첨 꺼짐(`chain.rs:1902` `next_protocol() >= 3`), 등록 상한 없음(`forks.rs:19`는 v2 업그레이드 때만 적용). mainnet-launch.md에는 업그레이드 단계도 없음 | **CRITICAL** |
| **G2** | 잔액 없는 거래: 지갑이 팁 0 (12:106 (1), 13 E1) | FFI `fee_caps`가 팁을 1 gwei로 고정(`crates/ffi/src/lib.rs:550-566`), `prepare`가 모든 거래에 적용(`:1068-1088`). 확장 wasm도 같음(`crates/wasm/src/lib.rs:28-32`). 실행 계층은 `gas_limit × max_fee.exec`만큼 잔액을 요구(`execution/src/block.rs:273-276`). 그래서 `prepare_register_node`(`ffi:1141-1158`, gas 400k)가 잔액 0에서 실패함. CLI만 `--tip 0` 지원(`main.rs:2346-2348`) | **CRITICAL** |
| G3 | 업그레이드 7일 예고, 앱이 미리 알림 (12:115) | `chain.rs:1467-1475`: 예고가 1 에포크 | HIGH |
| G4 | 레지스트라 키를 Secure Enclave·서명 전용 Mac에 (12:115) | `main.rs:329,725,1417` 파일 키. mainnet-launch.md:21도 "검증자 1번 Mac"에 둠 | HIGH |
| G5 | 재현 가능한 빌드와 증명 프로그램 ID 고정 (15:161, 12:115) | `scripts/prover-program.sh`에 `--remap-path-prefix` 없음. 앱·노드 릴리스 스크립트에도 없음 | HIGH |
| G6 | 인계 명단 바인딩, 예비 키 강화, 레드팀 09-29 수정 | 진행 중, 미병합 | HIGH |
| G7 | 릴리스 로그와 빌더 공동서명 (공급망) | 진행 중 | HIGH |
| G8 | 그룹 대비 제네시스 필드 (13:135 "메인넷 제네시스에 미리") | 제네시스를 한 번 만들면 되돌릴 수 없어서 나중에 넣을 수 없음. 진행 중 | HIGH |
| G9 | 교차 모델 감사 1회 무결점 (12:106 (3)) | 합의·발행·대기열 코드가 아직 동결 전(G1·G2·G6 미해결) | HIGH |
| G10 | 에포크당 등록 상한이 제네시스부터 있어야 함 | G1과 같은 원인(`registry.rs:61-70`이 v2 전용) | HIGH |
| G11 | 레지스트라 철회 경로 (12:116) | `Activation.registrar: Option`은 교체만 가능하고 없앨 수 없음 | MEDIUM |
| G12 | 첫 보상 알림, 공개 네트워크 화면 (15:69-73) | 없음 | MEDIUM |
| G13 | 이름 서비스와 금고 UI ("메인넷 전 만들 것", 12:70) | `contracts/src`에 이름 서비스 없음. 금고는 UI 없음 | MEDIUM |
| G14 | k-of-n 복구 FFI와 `addOwner` (12:19) | `ffi/lib.rs:1560` | MEDIUM |
| G15 | 긴 시뮬레이션 soak와 퍼징·Quint 완성 (12:16-17) | 기본 장애 주입과 파서 타깃 5개는 추가됨. 긴 soak·실제 60초 퍼징·BAL/증명 타깃·Quint는 남음 | MEDIUM |
| G16 | 라이선스 파일 (소스 공개와 동시) | 루트에 없음 | MEDIUM |
| G17 | 제재 명단·지역 차단 (직접 호스팅할 경우) | 없음 | MEDIUM (호스팅할 때만) |
| G18 | 에라 묶음 서명 (B3) | `era.rs:34` | LOW |
| G19 | 운영자 색인 (`is_operator` 전체 훑기) | `rewards/lib.rs:337` | LOW |

G1은 제네시스 직후 서명 업그레이드 2번(프로토콜 2, 그다음 3)으로 우회할 수 있습니다. 하지만 이 절차는 런북에도 리허설(`mainnet-rehearsal.sh`)에도 없습니다. 게다가 G3을 고쳐 예고를 7일로 늘리면, 메인넷의 첫 7일 이상이 증명 보상과 등록 상한 없이 돌아갑니다. 제네시스 매개변수로 넣는 것이 맞습니다.

---

## 3. 설계에 없이 구현된 것 (드리프트)

| 구현 | 위치 | 비고 |
|---|---|---|
| 머클 청구·`TokenBatch` 도구 | `contracts/src/MerkleDistributor.sol` | 17-2에는 있지만 12:70 "만들 것" 목록에는 없음. "에어드롭 안 함"(12:120)과 헷갈리지 않게 "사용자용 도구"라고 적어 두는 편이 좋음 |
| 등록 상한 값 16 | `registry.rs:62` | 설계 문서에 숫자가 없음 |
| 시뮬레이션 장애 `SplitList`, `SlowDisk` | `tests/sim.rs:63-74` | 12:16 상태 칸에 반영 안 됨 |
| 소스 공개 스캔을 gitleaks와 IP 치환으로 | `scripts/prepublish.sh` | 설계는 opensource-sanitizer(12:97) |
| 업그레이드 예고 = 1 에포크 | `chain.rs:1467` | 설계는 7일 |
| 예비 키 최대 3개 (창업자 등록 Mac과 별도) | `rewards/lib.rs:387` | 15:270 "사용자 확인 필요" 그대로 열려 있음 |
| 보상 CSV 문구가 "proofs earned"라고만 함 | `SimpleDashboard.swift:320` | 노드 몫(`kind: node`)도 포함되므로 문구가 부정확함 |

---

## 4. 설계 문서끼리 어긋나는 곳

1. **런치패드.** 12:114와 12:70(새 원칙)은 "메인넷에서도 제공". 같은 12:68과 12:70 뒤쪽 "이전 원칙"은 "만들지 않음". 13:143은 "메인넷 공식 런치패드 하지 않음, 테스트넷 데모만".
2. **발행 식.** 15:167 `I(h) = 1 AETH >> (h / 31,536,000)`은 1년 반감 식인데, 15:305-316과 코드(`rewards/lib.rs:125`)는 연 15% 감쇠에 0.1 바닥.
3. **발행 시작 시점.** 12:102 "발행을 끈 채 시작"과 12:119 "발행까지 운영자 n/16"은 12:103, 15 전체, mainnet-launch.md:3 "첫 블록부터 분배"와 충돌.
4. **소스 공개·감사 조건.** 12:97(30일 무리셋 뒤 공개), 12:102(30일, 버그 바운티), 12:108(감사 **연속 2회**)과 13 D1·D2가 한쪽이고, 12:106(메인넷과 **동시** 공개, 감사 **1회**)과 13:74·82가 다른 쪽.
5. **저장 수치.** 12:107 "빈 블록 0.5 KB, 1년 16 GB"와 13:16·41 "1.5 KB, 47 GB(측정 1,553 B)".
6. **예비 키 착석.** mainnet-launch.md:13 "예비 키 자동 착석", :108 "예비 키 3개가 자리를 지킨다", :109 "`aether_handoff` 7멤버(4+3)"는 15:262("4석이 서 있으면 하나도 안 넣는다"), reserve-keys.md:11, `mainnet-rehearsal.sh:204-213`과 정반대. 12:105의 원래 문장도 옛 규칙.
7. **레지스트라 키 위치.** mainnet-launch.md:21-22(검증자 1번 Mac의 파일과 `.p8`)와 12:115(서명 전용 Mac의 Secure Enclave).
8. **조건 번호 참조.** mainnet-launch.md:5 "12-launch-plan의 빠른 메인넷 조건(E1–E7)"인데, E1~E7은 13-roadmap §E에 있고 12는 (1)~(7)을 씀.
9. **프로토콜 업그레이드.** 15:90 "메인넷은 제네시스부터 이 규칙(업그레이드 불필요)"과 15:303 "지금은 업그레이드로 켠다". mainnet-launch.md에는 이 단계가 없음(G1).
10. **이미 끝난 일이 남은 일로 적힘.** 15:299의 DeviceCheck 토큰 코드는 이미 구현됨(`NodeController.swift:199-217`).
11. **네트워크 문서.** 08:30-34·09:51의 Pkarr 프로젝트 부트노드 목록은 실제 구현(노드별 DHT 레코드)과 다름. 08:47 "청크 = 1,000블록"은 B3의 에라 8,192블록과 다름. 08:48 "iroh-blobs 시딩"은 13:44 "iroh-blobs 쓰지 않음"과 충돌.
12. **12:16 1단계 상태 칸.** "장애 주입은 Partition/Isolate뿐"이라고 적혀 있지만 SplitList와 SlowDisk가 이미 추가됨.

---

## 5. 메인넷까지 남은 일 (우선순위순)

### 코드
1. **[CRITICAL] G1:** network.json·`ChainConfig`에 제네시스 프로토콜(또는 제네시스 활성화 일정)을 추가합니다. 프로토콜 2·3, registry v2 상한을 높이 0부터 적용하고 리허설 PASS 항목에 넣습니다.
2. **[CRITICAL] G2:** FFI·wasm `fee_caps`를 고칩니다. 잔액이 없거나 기본 수수료가 0이면 팁 0, `max_fee.exec = base × 2`로 합니다. 앱 등록 경로(`prepare_register_node`)를 잔액 0으로 시험합니다.
3. G6·G7·G8: 진행 중인 브랜치를 병합합니다(인계 명단 바인딩·예비 키 강화, 레드팀 09-29, 릴리스 로그·공동서명, 그룹 번호·위원회 크기·에포크 BLS 난수 제네시스 필드).
4. G3: 업그레이드 최소 예고를 7일로 하고, 앱에 예약된 업그레이드 알림을 넣습니다(G1 이후에 해야 메인넷 초기가 막히지 않음).
5. G5: `--remap-path-prefix`와 고정 툴체인으로 증명 프로그램 ID·노드·앱을 재현 가능하게 빌드합니다. 앱에 Sparkle 서명 지문을 표시합니다.
6. G4·G11: 레지스트라 서명을 Secure Enclave로 옮기고(서명 전용 Mac 도우미), 위원회 업그레이드로 레지스트라를 철회하는 경로를 추가합니다.
7. mainnet-launch.md와 `mainnet-rehearsal.sh`에 프로토콜·상한·잔액 0 거래 점검을 추가합니다.
8. G12: 첫 보상 알림, 공개 네트워크 화면(`aether_rewardStatus` 기반).
9. G13: 이름 서비스 컨트랙트, 금고·잠금 UI(설계 원칙상 메인넷 전 목록).
10. G14: k-of-n 복구 FFI와 `addOwner`를 앱에 노출합니다.
11. G15: 긴 시뮬레이션 soak와 퍼즈 스모크를 실행하고, BAL·증명 타깃과 Quint 명세를 완성합니다.
12. G16: 루트에 LICENSE 파일 두 개를 둡니다.
13. 문서 정리: 4절의 12개 항목, 특히 6번(예비 키 착석)과 2번(발행 식)은 운영 사고로 이어질 수 있습니다.

### 운영과 사용자 조치 (🧑)
1. 서명 전용 Mac을 준비하고 앱 인증서·Sparkle 키·레지스트라 키·예비 키를 옮깁니다. GitHub·Apple 계정에 하드웨어 키 2단계를 겁니다(12:115).
2. 서로 다른 제네시스 검증자 기계 3곳 이상(이 Mac, poc-m3, 외부 Mac)을 확보하고 poc-m3 가입을 결정합니다(12:106 (6), 13 A2·E6).
3. 최종 코드로 테스트넷 7일 무중단을 확인합니다(12:106 (2)).
4. 결정할 것:
   - 예비 키 3개를 창업자 등록 Mac과 별도로 둘지(15:270).
   - 이력 v2가 빈 블록 증명 발행을 없애는 것(13:43).
   - 가지치기 보존 기간과 메모리 부담(13:44).
5. DeviceCheck `.p8` 키, Chrome 웹 스토어 개발자 계정, TestFlight 공개 링크.
6. 제네시스 실행 직전 소리 내어 확인하는 절차(mainnet-launch.md §4).

### 감사·법률
1. 합의·발행·대기열 코드를 동결한 뒤 교차 모델 감사 1회 무결점(12:106 (3)). 12:108의 "연속 2회"와 어느 쪽을 따를지 먼저 정해야 합니다.
2. 비밀·개인정보 스캔(`scripts/prepublish.sh`)을 실행하고 결과를 기록합니다(12:106 (4)).
3. README·공지 문구 법률 재검수(12:106 (5)).
4. **런치패드를 메인넷에서 제공한다면 직전에 변호사 검토가 필요합니다**(12:114, 🧑). DEX·런치패드 컨트랙트는 다른 저장소에 있어 이번 분석에서 공정 출시 장치를 확인하지 못했습니다.
5. 메인넷 뒤: 공개 버그 바운티 30일(13 D4).

---

**다음 조치:** Match Rate가 70% 미만이라 설계와 구현을 맞추는 작업이 필요합니다. G1·G2는 코드를 고쳐야 하고, 4절 항목은 설계 문서를 고쳐야 합니다. 둘을 나눠 `/pdca iterate aether-node`를 돌리는 것을 권합니다. 파일은 하나도 고치지 않았습니다.
