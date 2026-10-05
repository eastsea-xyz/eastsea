# 29. 로그인 없이 돌아오는 노드 (2026-10-05)

## 사건

2026-10-05 07:50 이 Mac이 재부팅됐고, 10:30 사용자가 로그인하기 전까지 아무것도 돌아가지
않았다. 테스트넷 검증자 4대(`~/Library/LaunchAgents/com.pipln.aether.testnet.v1..v4.plist`)도
앱의 노드도 모두 **사용자 LaunchAgent / 앱 자식**이라 GUI 로그인 세션 안에서만 시작한다.
검증자 Mac이 밤새 재부팅되면(macOS 자동 업데이트, 정전) 사람이 로그인할 때까지 자리를 비운다.
메인넷에서 이것은 보상 손실이고, 4석 중 2석이 같이 재부팅되면 체인이 멈춘다.

목표: 노드(검증자·후보·팔로워 구분 없이)를 돌리는 Mac은 **아무도 로그인하지 않아도** 재부팅
후 노드를 되살린다 — macOS가 허용하는 한. 허용하지 않는 곳(아래 FileVault)은 사용자에게
정직하게 말한다.

## 조사: 로그인 전에 무엇이 돌아오는가 (macOS 15/26)

| 계층 | 언제 실행 | 우리에게 해당되는 것 |
|---|---|---|
| 시스템 LaunchDaemon(`/Library/LaunchDaemons`, `launchd`의 system 도메인) | 부트 직후, 로그인 전. 기본 root, plist의 `UserName`으로 사용자 계정 실행 가능 | **노드를 여기에 둔다** |
| 사용자 LaunchAgent(`~/Library/LaunchAgents`, GUI 세션) | 그 사용자가 로그인할 때 | 현재 테스트넷 검증자 4개. 사건의 원인 |
| 로그인 항목(`SMAppService.mainApp`) | GUI 로그인 시 앱 실행 | 현재 앱의 "로그인 시 열기" |

**SMAppService.daemon** (macOS 13+, ServiceManagement): 앱이 앱 번들 안
`Contents/Library/LaunchDaemons/<label>.plist`를 등록하면 launchd가 system 도메인에서
실행한다. 특징:
- plist는 앱 번들 안에 있어야 하고(앱이 서명된 채로 배포), `BundleProgram` 키로 번들
  상대 경로의 실행 파일(스크립트 포함)을 지정할 수 있다 — 번들의 절대 경로를 plist에
  박지 않아도 된다.
- 등록만으로는 `requiresApproval`: 사용자가 **시스템 설정 ▸ 일반 ▸ 로그인 항목**(macOS
  15+; "백그라운드 허용")에서 직접 켜야 `enabled`가 된다. 앱은
  `openSystemSettingsLoginItems()`로 그 패널을 열어 줄 수 있다.
- `RunAtLoad`/`KeepAlive`/`ThrottleInterval` 등 launchd 키는 등록된 plist에서도 그대로
  존중된다.
- 실행은 기본 **root**. `UserName` 키를 쓸 수 있지만 plist가 번들에 정적으로 들어가므로
  설치 시점에 사용자 이름을 쓸 수 없다 → 아래 "루트 스텁" 설계로 해소한다.
- `unregister()`로 깨끗하게 제거된다.

**FileVault** (`fdesetup status`): 데이터 볼륨 전체 암호화. 냉부팅·정전 후에는 잠금
화면에서 **사람이 한 번** 암호를 넣어야 볼륨이 풀린다. 그 전에는 `/Users/*` 전체(노드 데이터,
검증자 키, 마커 파일 모두)를 읽을 수 없다 — root도 못 읽는다. 이것은 우회할 수 없고, 우회를
시도하지도 않는다(설계상 예외 없음). 단, **macOS 소프트웨어 업데이트의 재시작**(인증된
재시작, authenticated restart)은 잠금을 한 번 풀고 재부팅하므로 로그인 없이 로그인 화면까지
도달한다 — 이 경우 데몬이 돌아온다. 2026-10-05 07:50의 재부팅이 업데이트 재시작이었다면
이 설계만으로 사건은 일어나지 않았다.

**정전 후 자동 켜기** (`pmset -g`의 `autorestart`): 시스템 설정 ▸ 배터리(노트북)/에너지
(데스크톱)의 "정전 후 자동으로 켜기". 앱은 **읽기만** 하고 변경은 사용자 몫이다(관리자 권한).
FileVault가 꺼져 있고 이 옵션이 켜져 있어야 정전 후 무인 부팅이 완성된다.

**노드가 로그인 전에 필요로 하는 것**: 데이터 디렉터리와 그 안의 파일들뿐이다.
- 검증자 신원: `<data>/validator.key`, `validator.pub.json`, `node-account.key` —
  0600 일반 파일(`supervisor.rs`의 `KEEP_ACROSS_NETWORKS`). 키체인 아님.
- 임계값 셰어·네트워크 파일: 역시 `<data>` 안의 파일.
- 지갑 키(Secure Enclave, `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`,
  `EnclaveKey.swift`)는 **노드가 필요로 하지 않는다** — 노드 서명은 validator.key로 한다.
  따라서 로그인·잠금 해제 없이 노드는 검증을 계속할 수 있다.
- 예외 하나: **DeviceCheck 재인증 토큰은 앱만 만들 수 있다**
  (`NodeController.refreshDeviceToken`가 시간당 하나를 `<data>/devicecheck-token`에
  남긴다). 무인 상태가 길어지면 일일 재인증이 빠지고, 그 기간 보상 적립이 멈춘다
  (`candidate.rs`: "earns nothing until it re-attests"). **합의·투표는 계속**된다 — 사건의
  피해(체인 정지)는 막고, 보상은 사람이 다시 앱을 열면 이어진다. 설정 문구에 담는다.

## 설계

### 원칙

1. **한 데이터 디렉터리엔 한 노드** — 이미 `run.lock`(flock)이 지킨다(red team #12). 앱이
   시작한 노드와 데몬이 시작한 노드가 절대 동시에 돌지 않는다. 늦은 쪽은 종료 코드 7
   (`EXIT_LOCKED`)로 "이미 실행 중"을 알리고 조용히 물러난다.
2. **사용자 승인은 한 번** — 시스템 설정 승인은 귀중하다(사람이 클릭해야 한다). 켜기/끄기
   동작은 승인을 유지한 채 마커 파일로 제어한다. "끄기"(`unregister`)는 설정을 끌 때만.
3. **root로 돌리는 것은 스텁뿐** — 노드는 항상 사용자 계정으로. 데이터 파일 소유권이
   섞이면(로그·체인 데이터가 root 소유가 되면) 앱 노드가 나중에 쓸 수 없다.
4. **무인 우선** — "재시작 후에도 유지"를 켠 Mac에서는 배터리 규칙(`onlyOnPower`)보다
   노드 생존이 우선한다. 설정 문구로 고지한다.

### 앱 경로 (일반 사용자의 Mac)

```
부트 → launchd(system) → com.pipln.eastsea.node (BundleProgram: 스텁, root)
  스텁: /Users/*/…/EastSea/node/unattended.plist 마커를 30초마다 찾는다
        (FileVault 잠금과 옵트인 안 함을 구분할 수 없어, 없어도 포기하지
        않고 조용히 기다린다 — exit 0으로 물러나면 KeepAlive가 재시작하지
        않아 그 부팅 내내 데몬이 죽어 있게 된다)
  발견 → sudo -u <마커의 사용자> Helpers/eastsea-node-wrapper.sh
  래퍼: caffeinate -s -w $$ (무인 검증 Mac은 자지 않는다)
        $$를 <data>/unattended.pid에 기록 (앱이 정지시킬 수 있게)
        exec <binary> run --data … [마커의 인자] >> node.log   ← 사용자 계정
```

- **마커 `unattended.plist`** — 앱이 `<data>/`에 쓴다: 사용자, 앱 번들 경로, 노드 바이너리
  경로, `argv`(앱이 노드를 시작할 때 쓰는 인자 그대로, 단 `--exit-with-parent`는 빼고),
  증명 주소(있으면 `AETHER_PROVE` 환경변수). **인자의 단일 원본**: 앱 `start()`가 같은
  순수 빌더(`UnattendedDecision.nodeArgv`)로 앱쪽 argv를 만들므로 앱 노드와 데몬 노드의
  구성이 어긋나지 않는다. 앱이 설정을 바꿀 때마다 다시 쓴다.
- **앱이 열릴 때** — RPC(127.0.0.1:18545)에 먼저 탐침: 이미 노드가 답하면(데몬 노드)
  **붙는다(attach)**. 두 번째 노드를 시작하지 않는다. 앱이 먼저 시도했다가 종료 코드 7로
  끝나는 경우도 attach로 전환한다. attach 중에도 지갑 경로·높이·투표 상태·DeviceCheck
  토큰 갱신은 모두 동일하게 동작한다(노드는 같은 포트를 쓴다).
- **attach 중 노드가 멈추면(stall)** — 앱은 그 노드를 TERM(pid 파일)하고 스스로 시작한다:
  감시 계층(docs/design/24)은 소유 여부와 무관하게 동작한다.
- **앱 종료·로그아웃** — attach 중이면 그냥 분리만 한다. 데몬 노드는 계속 돈다(그것이 목적).
- **노드 끄기(토글)** — 마커를 지우고, 돌고 있는 데몬 노드가 있으면 TERM한다. 스텁은 마커가
  사라졌음을 보고 조용히 기다린다(다시 나타나면 그것을 따른다).
- **설정 끄기** — `unregister()`. 시스템 설정에서 사라진다.
- **승인 대기** — 등록 후 상태가 `requiresApproval`이면 설정 화면에 "시스템 설정에서
  허용해 주세요" + 그 패널을 여는 버튼을 보여 준다.

### 기본값 (설정 "재시작 후에도 이 Mac의 노드 유지")

- 이 Mac이 **투표 세트에 있거나 등록된 후보**(`votingNodeStatus.registered`)면 **기본 켜짐**
  — 일반 문장으로 왜 그런지 설명한다. 사용자가 한 번이라도 직접 토글하면 그 선택을 영원히
  존중한다(`unattendedUserChose`).
- 팔로워(미등록)는 기본 꺼짐.

### 정직한 안내 (설정 화면, `pmset -g` + `fdesetup status`)

| FileVault | autorestart | 문장 |
|---|---|---|
| 켜짐 | — | "정전이 나면 이 Mac은 잠금 화면에서 기다려요. 한 번 잠금을 풀면 노드가 저절로 돌아옵니다. macOS 업데이트로 재시작하면 잠금 없이 돌아옵니다." |
| 꺼짐 | 켜짐 | "정전이 나도 이 Mac은 저절로 켜지고, 로그인 없이 노드가 돌아옵니다." |
| 꺼짐 | 꺼짐 | "시스템 설정 ▸ 배터리(에너지)에서 '정전 후 자동으로 켜기'를 켜 주세요." |
| 알 수 없음 | — | "전원 설정을 읽을 수 없어요." (앱이 바꿀 수 있는 것이 아니다) |

### 운영 스크립트 (창업자의 검증 Mac — 테스트넷 지금, 소크·메인넷 나중)

`scripts/install-validator-daemons.sh`: 기존 사용자 LaunchAgent plist(또는 새로 생성)를
`/Library/LaunchDaemons` 항목으로 바꾼다 — `UserName`을 사용자 계정으로, `--exit-with-parent`
제거, caffeinate·KeepAlive·ThrottleInterval 유지. 하는 일을 모두 출력하고 sudo로 실행한다
(이 스크립트 자체는 개발 중 건드리지 않는다; `--dry-run`은 임시 디렉터리에 쓴다).
`--uninstall` 제공. 앱 경로와 달리 여기선 plist를 직접 쓰므로 `UserName` 키로 곧바로 사용자
계정 실행 — SMAppService 번들 plist의 정적 제약이 없다.

## 하지 않는 것

- FileVault 잠금을 우회하는 어떤 시도(블록 수준 대기·NVRAM 키 보관 등). 불가능하거나
  위험하고, 정직한 안내가 답이다.
- 무인 상태에서 DeviceCheck 토큰을 노드/스크립트가 생성하는 것. Apple 정책상 앱만 만들 수
  있고, 그렇지 않은 경로는 하자가 된다.
- 원격에서 사용자의 데몬을 조작하는 기능.

## 시험

- Swift(순수 로직, `Tests/unattended`): 기본값 규칙(등록됨→켬, 사용자 선택 우선),
  attach 판정(RPC 생존/종료 코드 7), pmset/fdesetup 출력 파싱, 안내 문장 행렬,
  앱·데몬 argv 동일성(`--exit-with-parent` 제외).
- Rust(`crates/node/tests/run_lock.rs`): 실제 `aether run` 프로세스 두 번째 실행이
  `run.lock`을 잡지 못하고 종료 코드 7로 조용히 끝나는 것 — 앱 노드와 데몬 노드가 절대
  이중 실행되지 않음의 근거.
- 스크립트(`scripts/tests/test-install-validator-daemons.sh`): `--dry-run`이 임시
  디렉터리에 올바른 plist(UserName, exit-with-parent 제거, KeepAlive)를 쓰는 것,
  `--uninstall` 계획 출력.
- 빌드 확인: `scripts/verify.sh swift-pure xcodegen xcode-mac`(각각) + 건드린 Rust 크레이트
  `cargo test`.

## 한계 (사용자에게 그대로 말한다)

1. **FileVault 냉부팅/정전**: 사람이 잠금을 풀어야 한다. 그 사이 노드는 없다. (macOS 업데이트
   재시작은 제외 — 인증된 재시작이 한 번 풀어준다.)
2. **정전 후 자동 켜기**는 사용자가 시스템 설정에서 켜야 한다(관리자).
3. **무인 상태의 보상 적립**: DeviceCheck 재인증이 멈추면 적립이 멈춘다(투표는 계속).
4. **시스템 설정 승인**: 첫 등록은 사람이 한 번 "허용"해야 한다. 그 전엔 데몬이 돌지 않는다.
