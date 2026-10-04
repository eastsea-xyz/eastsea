# 25부속: 개명 진행 기록 (2026-10-04, glm/rename-1)

[25-rename.md](25-rename.md) 1·2·3·6단계 실행 기록. 리드 검토용: **바꾼 것 / 일부러 둔 것 / 이유**를
영역별로 적는다. 표기 규칙: Aether→EastSea(동해), AETH→Doubloon(DBLN). 합의·통신·크레이트 이름(4·5단계)은 건드리지 않는다.

## 분류 기준

- **(a) 사용자 표시 문구** — 문장·라벨·파일 이름 등 사람이 보는 것 → 바꾼다
- **(b) 식별자(유지)** — 크레이트명, `aether_*` RPC, 시그니처 도메인, 바이너리명(`aether`, `aether-agent`,
  `aether-prover`), `window.aether`/EIP-6963 표면, env(`AETHER_*`), 내부 색상·타입명, 빌드가 쓰는 경로 → 둔다
- **(c) 번들·서비스 ID** — `com.pipln.aether*` → `com.pipln.eastsea*` (단, 실행 중인 테스트넷 잡은 예외)

## 영역별 분류표

### 1. 지갑 앱 (apps/wallet) — 1·2·3·6단계

| 항목 | 분류 | 조치 |
|---|---|---|
| `Brand.swift` (project/coinName/coinTicker) | (a) | 이미 EastSea/동해/Doubloon/DBLN — 그대로 사용 |
| 표시 문자열 잔여 `"AETH"` (AgentWalletPanel 등) | (a) | `Brand.coinTicker`로 교체 |
| 기본 저장 파일명 `aether-rewards.csv` (ProverMenu, SimpleDashboard) | (a) | `eastsea-rewards.csv` |
| CSV 헤더 `amount_aeth` (NodeController) | (a) | `amount_dbln` (소비자 없음 확인) |
| 데이터 폴더 `…/Application Support/Aether/node` | (c)→(a) | `…/EastSea/node` + 최초 실행 이행(move, 실패 시 copy; 옛 데이터 삭제 안 함) |
| `…/Aether/update-state.json` (Sparkle 상태) | (c)→(a) | `…/EastSea/update-state.json` + 이행(copy) |
| `…/AetherWallet/enclave-key.dat`(·시뮬레이터 키) | (c)→(a) | `…/EastSeaWallet/` + 이행(copy, 옛 파일 유지) |
| `…/Aether/agent` (AgentWalletPanel 읽음) | (b) | **둔다** — `aether-agent` CLI(apps/agent)가 같은 경로를 쓰며 CLI는 이번 단계에서 개명 안 함 |
| UserDefaults 도메인 (번들 ID 변경으로 리셋) | (c) | 최초 실행에 `com.pipln.aether` 도메인 키를 새 도메인으로 1회 복사 |
| `Terms.version` 4/5 → 5/6 (재동의) | (a) | 바꿈 (확장 `TERMS_VERSION` 3→4 도 함께) |
| 티커 표기 "test AETH" 등 | (a) | 이미 `Brand.coinTicker` 경유 — 결과적으로 "test DBLN" |
| "Connected to EastSea" (네트워크 카드) | (a) | 테스트넷에서 "Connected to EastSea testnet"으로 |
| `aether://` URL 스킴 (pay/call/connect/tx) | (b)+(a) | 스킴 등록은 `eastsea`+`aether` 둘 다, 파싱도 둘 다 받는다. 옛 링크 호환 유지 |
| 릴리스 산출물 `Aether.app`, `Aether-<v>.dmg` | (a) | `EastSea.app`, `EastSea-<v>.dmg` (스크립트·매니페스트 동기화) |
| 번들 ID `com.pipln.aether`(·`.ios`, URL name `.pay`) | (c) | `com.pipln.eastsea` 계열로 |
| `PRODUCT_NAME: Aether` (project.yml 양쪽 타깃) | (a) | `EastSea` (DMG·메뉴·앱 이름) |
| xcodeproj/타깃/스킴 이름 `AetherWallet`(·`IOS`), `AetherWalletApp`, `AetherWallet.xcodeproj` | (b) | **둔다** — 빌드 경로 식별자. 스크립트·verify.sh가 참조 |
| `WAETH` / "Wrapped AETH" (TokenGuard 고정 토큰) | (b) | **둔다** — 온체인 컨트랙트 상수(823B…WAETH). 5단계에서 컨트랙트와 함께 |
| `aether_*` RPC 호출, `AETHER_PROVE` env, 헬퍼 `aether`/`aether.prev`/`aether-agent` | (b) | 둔다 |
| `Color.aether`, `EarningsText.aeth`, `Wei.from(aeth:)` 등 내부 식별자 | (b) | 둔다 (화면에 안 나옴) |
| 주석 속 "Aether" (기능 설명) | (b) | 원칙적으로 둔다. 경로·파일명이 바뀐 주석만 정정(ResizeBenchmark 실행 명령 → EastSea.app/…/EastSea) |

### 2. 브라우저 확장 (apps/extension) — 1단계

| 항목 | 분류 | 조치 |
|---|---|---|
| manifest.json name/short_name/description | (a) | 이미 EastSea Wallet — 확인만 |
| popup.js / background.js 표시 문구 "AETH balance…" | (a) | DBLN(`Brand.coinTicker`)으로 |
| `TERMS_VERSION` 3 | (a) | 4로 |
| `window.aether`, 포트 `aether:*`, `aether-page`, `aether#initialized`, `isAether` | (b) | **둔다** — dApp 대면 API. 4단계에서 `window.aether`와 함께 결정 |
| EIP-6963 `rdns: com.pipln.aether` | (b) | **둔다** — dApp 식별자(e2e 시험도 검증). 4단계 후보 |
| `aether_*` RPC 메서드, `wasm/aether_wasm.js` 경로 | (b) | 둔다 |
| 배포 zip `dist/aether-extension-<v>.zip` | (a) | `eastsea-extension-<v>.zip` |
| README의 AETH/Aether 문장 | (a) | DBLN/EastSea로 (코드 식별자는 유지) |

### 3. 탐색기 (apps/explorer) — 1단계

| 항목 | 분류 | 조치 |
|---|---|---|
| `<title>Aether Explorer</title>`, meta description | (a) | EastSea Explorer |
| app.js 브랜드 라벨, "an Aether node" 문장, rpc.js 오류 문장 | (a) | EastSea로 |
| pages.js 금액 표기 `AETH` | (a) | DBLN |
| `formatAeth`/`AETH_DECIMALS` 등 내부 함수명, `aether_*` RPC | (b) | 둔다 |

### 4. 사이트·README·법무·출시 문서 — 1단계

| 항목 | 분류 | 조치 |
|---|---|---|
| site/index.html | (a) | 이미 EastSea/eastsea.xyz — 확인만 |
| README 6개 언어 "testnet AETH", "1 AETH a block" 등 | (a) | DBLN. `VotingRules.mainnetRewardsRule` 인용문은 앱 문구와 동일 단어로 |
| README `aether-agent`/`aether` CLI 명령·경로 | (b) | 둔다 |
| DISCLAIMER §2 AETH (영어·한국어) | (a) | DBLN — 법적 의미·문장 구조는 그대로, 티커만 |
| docs/launch/demo-and-show-hn.md 표시 문구·DMG 이름·Show HN 제목 | (a) | EastSea/DBLN. 도구 식별자는 유지 |
| docs/research/*, 루트 조사 문서 2개 | (b) | **둔다** — 당시 기록 |
| docs/design/* 본문의 Aether 식별자 | (b) | 둔다 (역사 문서). 추후 4·5단계에서 정리 |

### 5. 스크립트·서비스 — 2·3단계

| 항목 | 분류 | 조치 |
|---|---|---|
| 실행 중 테스트넷 잡 `com.pipln.aether.testnet.v1..v4`, `com.pipln.aether.soak.*` | (b) | **무조건 유지** — 라이브 체인 7780. 라벨 변경·재시작 없음 |
| testnet-launchagent.sh 라벨 | (b) | 유지. `AssociatedBundleIdentifiers`만 새 앱 ID로 |
| reserve-keys.sh 라벨 `com.pipln.aether.reserve.*` | (c) | `com.pipln.eastsea.reserve.*` — 신규 설치용(메인넷 전, 돌아가는 잡 없음) |
| package-mac.sh / release-mac.sh / builder-build.sh / repro-app-check.sh 산출물 이름 | (a) | EastSea.app / EastSea-<v>.dmg |
| release-approve.py 매니페스트 항목명 + 시험 | (a) | `EastSea.app/…`, `EastSea.dmg` (헬퍼 `aether`,`aether-agent` 항목명은 유지) |
| build-extension.sh zip 이름 + repro 문서 | (a) | eastsea-extension-<v>.zip |
| soak/monitor.sh 알림 제목 "Aether soak" | (a) | "EastSea soak" (라이브 복사본은 수동 갱신 필요 — README 참조) |
| start.sh / start.bat echo 문구 | (a) | EastSea |
| install.sh | (b) | **둔다** — v0.1.0 DMG(존재하지 않음)를 가리키는 유물. 리드 확인 후 삭제/갱신 권장 |
| devnet.sh / demo.sh / testnet*.sh 내부 경로·env (`AETHER_*`, `~/aether-testnet` 등) | (b) | 둔다 — 노드 바이너리·실행 체인과 결합 |
| `AETHER_*` env, `/aether-node` prefix-map(재현 빌드 상수), 크레이트 경로 | (b) | 둔다 |

### 6. 에이전트 CLI (apps/agent) — 1단계 범위 내 최소

| 항목 | 분류 | 조치 |
|---|---|---|
| 문장 속 "AETH" 한도 표기(Owner.swift 등) | (a) | DBLN |
| 문장 속 "Aether wallet for AI agents" | (a) | EastSea |
| `aether-agent` 명령·경로·`Application Support/Aether/agent` | (b) | 둔다 (도구 개명은 4·5단계) |
| Tools.swift 폴백 `/Applications/Aether.app/...` | (a) | `EastSea.app` 우선 + 옛 경로 유지 |

### 7. 명시적으로 손대지 않는 것

- `crates/` 전체(다른 엔지니어가 dkg*/supervisor/handoff 편집 중) — 이 브랜치에서 0바이트 변경
- `contracts/`, `benches/`, `fuzz/`, `spike/`, `legacy/`, `Cargo.toml` 크레이트명 — 4·5단계
- `AGENTS.md`, `agents/skills/aether-wallet/` — `aether-agent` 도구 문서, 도구와 함께 개명(이후 단계)
- 실행 중 테스트넷 잡/스크립트 라벨(위 표)

## 상태

- [x] 분류 작성 (2026-10-04)
- [x] 1. 지갑 앱 문자열·이행 — 커밋 006309a(이행), 8f6932c(문자열)
- [x] 2. 번들 ID·PRODUCT_NAME·URL 스킴 — 커밋 57c5e54, e10f244(pbxproj)
- [x] 3. 확장 — 커밋 4b4f95b
- [x] 4. 탐색기 — 커밋 88bb23d
- [x] 5. README·DISCLAIMER·출시 문서·사이트 — 커밋 fca1603 (site/index.html·manifest·brand.js·Brand.swift 는 이미 개명돼 있어 확인만)
- [x] 6. 스크립트·서비스 라벨 — 커밋 a891055
- [x] 에이전트 CLI 최소 — 커밋 0cb3de9
- [x] 검증 — swift-pure 14/14 OK(rename-migration 신설, release-approval 픽스처 동기),
      확장 93/93(wasm 빌드 후 시험, wasm은 삭제), 탐색기 43/43, release-approve unittest OK,
      bash -n 11개 스크립트 통과, xcodegen 재생성, xcode-mac BUILD SUCCEEDED(산출물 EastSea.app),
      xcode-ios BUILD SUCCEEDED, build-agent.sh 빌드 성공. release-mac.sh 는 무인자 실행이
      legacy 게시 경로(원래 동작)라 **실행하지 않고** 정적 확인으로 마쳤다.

## 결과 요약 (변경 규모)

- 지갑: 표시 문자열 13곳(AETH 폴백·CSV 헤더·저장 파일명 2·릴리스 산출물 3·링크 문구 2·네트워크 카드 1·약관 bump 2·주석 1) + 이행 3개 경로 + 번들 ID 4곳·PRODUCT_NAME 2곳·URL 스킴 이중 등록 2 plist
- 확장: 표시 문구 7문장 + TERMS_VERSION 4 + zip명 2 + e2e 기대문 5
- 탐색기: 제목·메타 2 + 화면 문장 7 + 금액 티커 3
- 문서: README 6개 언어·DISCLAIMER 영어·한국어의 티커 22줄, 데모/Show HN 문서 10곳, ops 문서 3곳
- 스크립트: 14개 파일 40줄(산출물 이름·라벨·알림 문구)
- 에이전트: 3곳(한도 문구·usage·network.json 탐색 경로)
