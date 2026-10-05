# 지갑(노드) 중심 체인에서 스마트 컨트랙트 서비스를 어떻게 출시하고, 사람들은 어떻게 찾는가 (2026-10-05)

> 창업자 질문 1: "지갑(노드) 중심 블록체인에서 스마트 컨트랙트 서비스를 어떻게 출시하고, 사람들은 어떻게 검색·발견하나?"
> 창업자 질문 2(추가): "서비스는 어떻게 홍보하고, 사람들은 어떻게 찾나?"
> 창업자 정정(필수 전제): **"출시하는 건 우리만이 아니다."** 제3자 빌더가 첫날부터 서비스를 올릴 수 있어야 한다.
> 그래서 이 문서의 설계는 **열린(permissionless) 레지스트리**가 기본이다. 누구나 게시(스팸 방지용 환불형 보증금), 중앙 승인 없음,
> 우리 앱도 투명한 규칙 밖의 특혜 순위 없음, 큐레이션은 "추천(featured)" 표시 칸과 안전 플래그에만.
>
> 근거 표기: 외부 사실은 `[번호]`로 맨 끝 출처 목록에 링크와 날짜(게시일, 없으면 확인일 2026-10-05)를 단다.
> 저장소 사실은 파일 경로로 단다(읽기 전용으로 확인, 브랜치 `lead-merge` 워크트리와 `glm-wallet-browser` 워크트리).

---

## 0. 결론 먼저 (10줄)

1. **선례 대부분은 "열린 게시 + 좁은 큐레이션" 쪽으로 수렴했다.** Farcaster/Base 미니앱은 심사 없이 도메인 서명 manifest만으로 검색에 오르고 사용량 신호로 순위를 매긴다 [F1][F2][B1]. Solana dApp Store는 온체인 NFT로 앱 정체성을 기록하고 수수료 0%지만 KYC+3~5일 심사를 둔다 [S1][S2][S3]. MetaMask Snaps는 위험 권한만 allowlist로 묶고 나머지는 permissionless로 풀었다 [M1].
2. **"게이트키퍼 없는 안전"은 전부 클라이언트 쪽 경고층으로 해결했다.** Phantom의 공개 GitHub blocklist [P1], Bluesky의 쌓을 수 있는(stackable) labeler [BS1], ar.io 게이트웨이별 자율 차단 [AR2], MetaMask+Blockaid 기본 경고 [M2]. 등록부(목록) 자체는 열어 두고, **보여 주는 방식**에서 막는다.
3. **우리의 결정적 이점은 "모든 사용자가 검증 노드를 들고 있다"는 점이다.** 다른 지갑은 서버(Blockaid, 순위 API)에 물어야 하는 것을, 우리는 내 Mac의 노드가 체인 이벤트와 로컬 계정 이력 인덱스(`crates/node/src/account_history.rs`)로 직접 계산할 수 있다. 검색·순위·경고를 **기기 안에서, 공개 규칙으로** 돌릴 수 있다.
4. **권고 설계:** `AppRegistry.sol`(누구나 게시, 환불형 보증금, 관리자 없음) + 프런트엔드 번들의 **content hash**를 온체인에 고정 + 지갑이 번들을 받아 해시를 검증한 뒤 `eastsea-app://<appId>/` 사설 스킴으로 격리 실행(이미 있는 `BundledPageScheme.swift` 패턴의 일반화) + `EastSeaNames` 텍스트 레코드 `app`과 manifest의 `name` 양방향 결합 + 기기 안 순위(등록된 Mac 노드 수, 소각된 수수료, 재방문 기준; 거래량 아님) + 쌓을 수 있는 플래그 목록(기본 목록은 공개 멀티시그 + 투명성 로그).
5. **Apple:** iOS 안의 HTML5 미니앱은 4.7로 허용되지만, 신고 기능·콘텐츠 필터·사용자 동의·**전체 목록 + universal link 색인(4.7.4)**·연령 처리 의무가 붙는다 [A1]. 3.1.5(v)는 "다른 사용자에게 다운로드를 권하면 코인 지급"을 금지하므로 **코인 리퍼럴 보상은 iOS에서 불가** [A1]. 거래소형 기능은 면허 지역 한정(3.1.5(iii)) [A1]. Coinbase는 2019년 iOS dApp 브라우저를 Apple 정책 때문에 뺐다 [A3]. → **Mac(Developer ID 배포)은 열린 레지스트리 전체, iOS는 같은 레지스트리를 쓰되 보수적 필터 + 4.7 의무 구현**.
6. **홍보:** 지갑 안 "새로 나온 / 추천" 칸, universal link·QR로 지갑 안에서 바로 열기, 서비스별 정적 랜딩 페이지(SEO), 조코헌트·Product Hunt·Show HN, 빌더 템플릿·해커톤. **코인 리퍼럴 보상은 하지 않는다**(Apple 3.1.5(v), 금융위 2025-12 레퍼럴=미신고 가상자산사업 판단 [K1], 법률 메모 3.6).
7. **AI 에이전트도 발견 채널이 된다.** 공식 MCP Registry(2025-09 프리뷰) [AG1], Coinbase x402 Bazaar(2025-09) [AG2] 선례처럼, `aether-agent`에 `apps_search`/`app_info` 읽기 도구를 넣으면 에이전트가 등록된 서비스를 찾고, **오너가 Touch ID로 승인한 payee에게만** 결제한다(기존 정책 그대로).
8. **가장 큰 위험 3가지:** (a) 미니앱 피싱 — Telegram 미니앱이 2026-04 FEMITBOT 사기망에 대규모 악용됨 [T5]; (b) 게시자 키 탈취 후 "악성 업데이트" — 업데이트 지연(timelock)과 사용자 재동의로 막아야 함; (c) 한국법상 우리 지갑이 거래·런치패드 앱을 "추천"하면 중개·알선으로 볼 위험 — 추천 칸에서 거래·발행 카테고리 제외.
9. **가장 작은 첫 단계:** 이번 주에 `docs/design/31-app-registry.md`(manifest 스키마+컨트랙트 인터페이스) 작성 → `BundledPageScheme`을 "해시 검증된 임의 번들" 스킴으로 일반화(익스플로러를 첫 번째 "레지스트리 앱"으로 이식) → 테스트넷에 `AppRegistry.sol` 배포 + `eastsea-app publish` CLI.
10. **제3자 첫날 출시와 우리의 genesis 원칙은 충돌하지 않는다.** 레지스트리 자체는 돈을 굴리지 않는(보증금만 보관) 작은 컨트랙트라 감사 범위가 좁다. DEX·런치패드는 여전히 컨트랙트 brake 전까지 genesis 밖(`docs/research/contracts-audit-2026-10-05.md`)이지만, **제3자가 자기 컨트랙트를 배포하고 레지스트리에 올리는 것은 막지 않는다** — 대신 지갑이 "brake 없음 / 감사 없음"을 사실대로 표시한다.

---

## 1. 출발점: 저장소에 이미 있는 것

| 구성 요소 | 상태 | 근거 |
|---|---|---|
| 인앱 브라우저(Explore 탭) | WKWebView. https와 번들 페이지만 허용, http·file 거부. 큐레이션 도메인(`eastsea.xyz`) 닮은꼴(편집거리 1, punycode) 경고 | `glm-wallet-browser` 워크트리 `apps/wallet/Sources/BrowserOriginPolicy.swift` |
| 번들 페이지 스킴 | `eastsea-page://explorer/…`를 `WKURLSchemeHandler`로 제공, 경로 탈출 거부, 응답마다 CSP | 같은 워크트리 `BundledPageScheme.swift` |
| EIP-1193 provider | 확장(`apps/extension/src/lib/methods.js`)과 인앱 브라우저가 같은 메서드 집합을 쓰도록 테스트로 고정 | `BrowserPolicy.swift`, `apps/extension/test/wallet-provider.test.mjs` |
| 사이트별 권한 | origin 단위 권한 저장 | `SitePermissions.swift` |
| 이름 서비스 | 3~32자, 1년 등록, 요금 전액 소각(3자 2 / 4자 0.5 / 5자+ 0.1 AETH), commit-reveal, **텍스트 레코드 최대 4개(키 `[a-z0-9-]` 32자, 값 128자)**, 역방향 레코드 | `contracts/src/EastSeaNames.sol`, `docs/design/26-name-service.md` |
| 로컬 계정 이력 인덱스 | `aether_accountHistory` RPC, `eth_getLogs`(최대 2,000 확정 블록) | `crates/node/src/account_history.rs`, `crates/node/src/rpc.rs` |
| Sybil에 강한 신원 집합 | 투표 노드 등록: Apple DeviceCheck 토큰, **기기 1대 = 투표 키 1개**, 무료·기계적 | `docs/design/14-registration.md`, `crates/node/src/devicecheck.rs`, `contracts/src/CommitteeRegistry.sol` |
| 에이전트 지갑 | MCP 서버 `aether`, 오너가 Touch ID로 payee·한도 승인, DEX 읽기 도구만 | `agents/skills/aether-wallet/SKILL.md`, `apps/agent/Sources/` |
| 재현 가능 빌드 연구 | 배지 "소스 일치"의 근거로 쓸 수 있음 | `docs/research/repro-builds-2026.md` |
| 공개 읽기 | 정적 자기검증 이력 + 교체 가능한 증명 서버. receipts/logs 커밋이 아직 없어 이벤트는 "노드 말을 믿는" 데이터 | `docs/research/public-read-access-2026-10-05.md` §0-5 |
| 컨트랙트 감사 | DEX·런치패드·TokenFactory는 소스와 brake가 없어 genesis 불가 | `docs/research/contracts-audit-2026-10-05.md` |

**빠진 것:** 앱 단위 정체성(지금 권한은 웹 origin에 묶임), 번들 무결성 검증, 앱 목록·검색, 플래그 체계, 에이전트용 앱 검색 도구.

---

## 2. 선례 조사 (2024-2026)

### 2.1 앱 배포·발견 플랫폼

**Solana dApp Store (Solana Mobile)**
- 게시하면 **Publisher NFT / App NFT(앱당 1회) / Release NFT(버전마다)**가 온체인에 생기고, 메타데이터와 APK는 ArDrive(Arweave) 또는 S3에 올린다. 약 0.2 SOL 필요 [S2][S4].
- Publisher Portal에서 **KYC/KYB** 후 제출, 심사 결과 3~5영업일 [S2]. 정책은 Google Play를 따르되 "dApp Store 사명과 충돌하는 부분은 예외"이고, 오인 유도·제3자 사칭·취약점 악용 코드를 금지, 신고 메일 운영, "조사·조치할 권리는 있으나 의무는 없다" [S3].
- `@solana-mobile/dapp-store-cli`로 업데이트 게시(APK 경로 → 포털이 릴리스 생성·온체인 트랜잭션) [S4].
- 수수료 0%. 앱 수 약 700(2026-03) → 1,000+(2026-06 초) → 1,561(2026-06-26) [S5]. 1,000개를 넘자 **발견이 문제**가 되어 주간 4개 테마 "dApp Spotlight", 평점·리뷰, AI 리뷰 요약을 도입 [S6].
- 시사점: 온체인 정체성(NFT)은 좋지만 **심사는 여전히 중앙**. 카탈로그가 커지면 큐레이션 칸이 따로 필요해진다.

**Telegram Mini Apps**
- 2024-07-22 MAU 9.5억 중 **5억 명 이상이 매달 미니앱 사용**, 2024-07-31 Mini App Store와 Web3 인앱 브라우저 출시 [T1][T2].
- 카탈로그 노출 조건: Main Mini App 설정, **Telegram Stars 결제 수용**, 고품질 미디어, 디자인 가이드 준수. 딥링크 `t.me/<bot>/<app>?startapp=…`, initData는 HMAC-SHA-256 또는 Ed25519 서명으로 검증 [T3].
- 2025-01 블록체인 가이드라인: **TON만 허용**, TON Connect만 허용, 타 체인 지갑 연결 보상 금지, 기존 앱은 2025-02-21까지 이전 [T4]. → 플랫폼이 체인을 고르는 **게이트키퍼 위험**의 실례.
- 성장 동력은 바이럴 리퍼럴: Notcoin 3,500만 명(2024-05), Hamster Kombat 3억 명 주장(2024-07) [T6][T7]. 그러나 Hamster Kombat은 에어드롭 전 **230만 계정을 봇·다계정 부정으로 제외**했고, 한 사람이 400개 계정을 한 거래소 주소에 연결한 사례도 있었다(2024-09) [T8].
- 2026-04 FEMITBOT: 미니앱 WebView 안에 가짜 거래소·가짜 수익 대시보드·브랜드 사칭·안드로이드 악성 APK를 띄우는 대규모 사기망 [T5]. **지갑 앱 내부 WebView가 "앱처럼 보이는 피싱"을 더 그럴듯하게 만든다**는 경고.

**World App mini apps**
- 개발자 포털에서 제출 → 검토 → 승인되면 World App 전체에 노출, 위반 시 "리뷰 팀 단독 재량으로 삭제" [W1][W2].
- 2024-10 출시, 2025-06 기준 **주간 순사용자 약 50만**, World ID 1,300만+ [W3]. Developer Rewards 파일럿: **World ID 인증 사용자 수 기준**으로 월 $300K WLD 분배(2025-04 시작) [W4].
- 시사점: "사람 인증된 사용자 수"를 보상·순위 신호로 쓰는 것이 Sybil 방어 핵심. 우리에겐 DeviceCheck 등록 노드가 비슷한 역할을 할 수 있다.

**Farcaster mini apps (구 Frames v2)**
- `/.well-known/farcaster.json` manifest, **도메인과 Farcaster 계정을 서명으로 묶는 `accountAssociation`**(JFS, payload `{domain}`), 필드: name(32자), iconUrl, homeUrl, primaryCategory, tags(5개), `noindex`, `requiredChains`(CAIP-2), `requiredCapabilities` 등 [F1].
- **심사 없음.** 검색 색인 조건: manifest 필수 필드, 개발 터널(ngrok 등) 도메인 제외, 한 번 이상 공유, 최소 사용량. 순위: 연 사용자 수, 컬렉션 추가 수, 최근 참여 트렌드 [F2].
- Frames 출시(2024-01-26) 1주 만에 DAU 5,000 → 24,700 [F3]. 그러나 2025-10 DAU 4~6만, Power Badge 기준 실사용 약 4,360명 추정(2차 출처) [F4]. 2025-12 창업자들이 "소셜 우선 4.5년, 우리에겐 안 됐다. 지갑이 성장 중"이라 하고 2026-01-21 Neynar가 Farcaster를 인수 [F5].
- 스팸 방어는 프로토콜이 아니라 **Neynar user score**(0~1, 주간 갱신, 2025-05 미니앱 사용자 반영해 재학습) 같은 외부 평판 점수 [F6]. OpenRank(EigenTrust 기반 개인화 순위)도 Farcaster용 API를 냈다 [F7].

**Base App (Coinbase)**
- 2025-07-16 Coinbase Wallet을 Base App으로 개명, 수백 개 미니앱 내장 [B2]. 미니앱은 Farcaster manifest 재사용, `npx create-onchain --mini` 템플릿(MiniKit) [B3].
- 검색: Base App에서 한 번 공유하면 약 10분 뒤 색인, `noindex`, manifest 무효화로 제거, **카테고리 순위는 7일 이동창 집계** [B1].
- 추천(featured) 칸은 별도 신청: 3초 내 로드, 지갑 자동 연결, 외부 리다이렉트 금지, 가스 대납 권장, 아이콘·스크린샷 규격. "충족해도 노출 보장 안 함" [B4].
- **2026-09-10 Coinbase Wallet로 다시 개명** — 암스트롱 CEO가 2026-03 "소셜 실험은 잘 안 됐다"고 말함, 사용자 수 비공개 [B5].
- 시사점: 열린 게시 + 사용량 순위 + 좁은 featured 칸이라는 **구조는 좋지만, 소셜 피드가 발견 엔진이 될 것이라는 가정은 두 번(Farcaster, Base) 실패**했다. 우리는 소셜 피드 대신 "지갑이 이미 아는 것(내가 돈을 보낸 곳, 내 이름, 내 노드)"을 발견 엔진으로 써야 한다.

**MetaMask Snaps**
- 키 관리 권한(`snap_getBip44Entropy`, `snap_manageAccounts` 등)을 쓰면 **승인된 감사인의 감사 필수**, 디렉터리 등재는 2인 승인 allowlist. `endowment:ethereum-provider`, `transaction-insight`, `snap_dialog` 등 "open" 권한만 쓰는 Snap은 **allowlist 없이 permissionless 설치** 가능 [M1].
- "allowlist 포함은 보증이 아니다"를 명시 [M1].
- 시사점: **권한 위험도에 따라 문턱을 달리하는 것**이 가장 실용적인 절충이다.

### 2.2 지갑 dApp 브라우저와 안전 목록

| 지갑/서비스 | 방식 | 근거 |
|---|---|---|
| MetaMask | 2024-02부터 Blockaid 보안 경고 기본 켜짐(여러 체인), 2024-07 Wallet Guard 인수(드레이너 휴리스틱) | [M2] |
| Blockaid | 월 7,500만 도메인 스캔, 월 160만 악성 dApp 차단, 신규 사기 평균 4분 내 탐지, 샌드박스에서 UI 퍼징·고액 지갑 모의 | [BL1] |
| Phantom | **공개 GitHub blocklist**(blocklist/fuzzylist/whitelist yaml), 커뮤니티 기여, 목록을 로컬 저장·주기적 갱신(브라우징 기록 수집 안 함), 15분 내 전파 | [P1] |
| Rabby | 서명 전 시뮬레이션으로 잔액 변화 표시, 컨트랙트별 허용 | [R1](2차 출처) |
| Scam Sniffer 2025 연간 | 피싱 손실 **$83.85M / 피해자 106,106명**(2024 대비 -83%), 최대 단건 $6.5M Permit 서명, 2025-08 EIP-7702 악성 서명 대형 2건 | [SS1] |

시사점: 손실 감소의 주원인은 **서명 시점 경고와 시뮬레이션**이다. 목록(어디서 왔나)보다 **행동(무엇을 서명하나)**을 보는 층이 더 강하다. 우리 지갑은 로컬 노드로 시뮬레이션(`eth_call` / 실행 엔진)을 직접 돌릴 수 있다.

### 2.3 이름 + 콘텐츠 해시 웹사이트

- **ENS + IPFS/eth.limo:** eth.limo는 ENS contenthash(IPFS/IPNS/Arweave/Swarm)를 풀어 HTTPS로 내주는 리버스 프록시. 2026 Q1 월 7,600만~8,600만 요청, 2026-02 Verizon DNS가 위협 피드 오탐으로 eth.limo 전체를 차단했다가 복구 [E1]. → **게이트웨이 하나는 단일 실패점**.
- **eth.casa(2026-07-30):** 서비스 워커가 ENS contenthash를 DoH와 RPC로 교차 확인하고 Helia `verifiedFetch`로 **모든 블록을 CID로 검증**. 단, 서비스 워커 자체가 온체인 증명이 없어 "*.eth.casa를 믿어야 한다" [E2]. → 우리는 검증기가 **지갑 바이너리 안**에 있으므로 이 마지막 신뢰 고리가 없다.
- **Arweave permaweb + ArNS:** 이름 → ANT(Arweave Name Token) → 트랜잭션 ID, 경로 manifest로 앱 묶음 [AR1]. **각 게이트웨이 운영자가 자기 정책으로 콘텐츠·이름·주소를 차단**(네트워크 전체 차단 목록 없음) [AR2].

### 2.4 탈중앙 발견과 순위

- **Nostr NIP-89:** 앱이 kind 31990으로 "이 이벤트 종류를 처리한다"고 공지, 사용자가 kind 31989로 앱을 추천, 클라이언트는 **내가 팔로우한 사람의 추천**으로 앱을 찾는다(web of trust) [N1]. NIP-78(kind 30078)은 앱별 임의 데이터 저장 [N2].
- **Bluesky/AT Protocol:** 누구나 피드 생성기를 만들고 `getFeedSkeleton`으로 게시물 URI 목록만 돌려준다(본문 채우기는 AppView) [BS2]. 모더레이션은 Bluesky 기본층 위에 **사용자가 고른 제3자 labeler를 쌓는** 방식, 도구 Ozone 오픈소스 [BS1].
- 시사점: **"순위 알고리즘도 사용자가 고르는 교체 가능한 부품"**으로 만들면, 우리 앱에 특혜를 주지 않는다는 약속을 구조로 보장할 수 있다.

### 2.5 순위 조작의 역사와 방어

| 조작 | 사례 | 방어 |
|---|---|---|
| 가짜 거래량(wash trading) | 2022-01 NFT 거래량의 약 80%, 2022년 평균 약 58%가 워시 트레이딩 추정 [D2] | 거래량을 순위 신호에서 빼거나 상한 |
| dApp 부스팅 서비스, 다계정 유도 보상 | DappRadar가 세 가지 조작 유형을 지목하고 필터·표시·삭제 [D1] | 고유 계정 수 대신 "비용이 드는 고유 신원" 수 |
| 다계정 파밍 | Hamster Kombat 230만 계정 제외, 한 사람 400계정 [T8] | 신원 비용(DeviceCheck, World ID), 자금 출처 클러스터링 |
| 소셜 그래프 봇 | Farcaster 실사용 추정 4,360명 대 DAU 4~6만 [F4] | EigenTrust/개인화 순위 [F7], 평판 점수 [F6] |
| 토큰 큐레이션 레지스트리(TCR) | adChain(2018) 등 참여 저조·담합 취약 [TC1] | **토큰 투표로 목록을 관리하지 않는다** |

---

## 3. 배포·발견 모델 비교

| 모델 | 신뢰 | 검열 저항 | 사용자 안전 | 개발자 마찰 | Apple 정책 적합 |
|---|---|---|---|---|---|
| A. 중앙 심사 스토어(World App, Solana 심사, Snaps 디렉터리) | 운영자 신뢰 | 낮음(단독 재량 삭제 [W2]) | 높음(사전 심사), 그러나 사후 사기 여전 | 높음(KYC, 3~5일) | 좋음(4.7 의무를 운영자가 이행) |
| B. 플랫폼 독점 체인(Telegram TON) | 플랫폼+체인 | 매우 낮음(체인 강제 [T4]) | 중간(FEMITBOT [T5]) | 중간 | 플랫폼이 Apple과 협상 |
| C. 도메인 서명 manifest + 사용량 색인(Farcaster, Base) | 도메인(DNS·TLS) + 클라이언트 | 중간(클라이언트가 색인 결정) | 중간(심사 없음, 외부 평판 점수 의존) | 낮음(템플릿, 10분 색인 [B1]) | 중간 |
| D. 온체인 등록 + 콘텐츠 해시(Solana NFT 메타데이터, ENS contenthash, ArNS) | 체인 + 해시 | 높음 | 해시가 "바뀌지 않음"만 보장, "선함"은 보장 안 함 | 중간(키·가스·저장소) | 번들 다운로드 실행은 4.7 범위여야 함 |
| E. 소셜 추천(NIP-89, Bluesky 피드) | 내 그래프 | 높음 | 그래프 품질에 의존 | 낮음 | 중간 |
| **F. 권고: D(정체성·무결성) + C(열린 색인) + E(교체 가능한 순위) + 쌓을 수 있는 플래그** | 체인 + 기기 안 검증 | 높음(등재는 막을 수 없음) | 표시층에서 경고·차단, 서명 시점 시뮬레이션 | 낮음(CLI 한 줄) | Mac 전체, iOS 4.7 의무 구현 |

---

## 4. EastSea 권고 설계

### 4.1 원칙

1. **등재는 막지 않는다. 표시는 사용자가 고른 규칙을 따른다.** 등재(registry) = 사실의 기록, 표시(view) = 기기 안의 정책.
2. **우리 앱(익스플로러, 이름)도 같은 레지스트리에 같은 방식으로 등록**한다. 기본 순위 규칙에 게시자 예외 없음. 지갑 하단의 고정 탭(익스플로러)은 "앱"이 아니라 지갑 기능으로 분리 표기.
3. **큐레이션은 두 군데뿐:** "추천" 칸(라벨 명시, 선정 기준 공개, 거래·발행 카테고리 제외) / 안전 플래그 목록.
4. **해시가 보장하는 것과 보장하지 않는 것을 화면에서 구분**한다. "이 번들은 게시자가 서명한 그 번들입니다" ≠ "이 앱은 안전합니다".

### 4.2 온체인 레지스트리 `AppRegistry.sol`

**저장(온체인, 최소):** 상태 수수료(`docs/design/27-state-fee.md`)를 고려해 큰 메타데이터는 이벤트와 해시로만.

```
struct App {
  address publisher;        // 게시자 키(EastSeaAccount 가능: Secure Enclave 패스키)
  address pendingPublisher; // 2단계 이전(이름 서비스와 같은 propose/accept)
  bytes32 manifestHash;     // 현재 릴리스 manifest의 sha256
  bytes32 bundleHash;       // 프런트엔드 번들(결정적 tar 또는 CAR)의 sha256
  uint64  activatesAt;      // 업데이트가 효력을 갖는 시각(업데이트 지연)
  bytes32 prevManifestHash; // 지연 기간 동안 사용자에게 보여 줄 이전 버전
  uint96  bond;             // 잠긴 보증금
  uint64  unlistedAt;       // 0이면 등재 중
}
appId = keccak256(abi.encode(publisherAtCreation, slug))  // 게시자 키를 옮겨도 불변
```

**함수:** `publish(slug, manifestHash, bundleHash) payable` / `release(appId, manifestHash, bundleHash)` / `unlist(appId)` / `withdrawBond(appId)`(unlist 후 30일) / `transferPropose/Accept` / `cancelRelease(appId)`(지연 기간 중 게시자가 취소).
**이벤트:** `Published`, `Released(appId, version, manifestHash, bundleHash, activatesAt)`, `Unlisted`, `Transferred`. 노드 인덱서는 이 이벤트만 읽으면 된다.

**보증금(anti-spam):**
- 게시 시 보증금 예: 1 AETH(환불형) + 소각 수수료 0.1 AETH(이름 서비스 5자+와 같은 값, 비환불). 값은 메인넷 전 다시 정한다.
- **누구도 보증금을 몰수할 수 없다**(관리자·투표 슬래싱 없음). 몰수를 넣으면 몰수 권한자가 곧 중앙 심사자가 되고, TCR처럼 담합 대상이 된다 [TC1]. 보증금의 역할은 "스팸 1만 건을 올리려면 1만 AETH를 30일 이상 묶어야 한다"는 **기회비용**뿐이다.
- 보증금 회수 30일 대기는 "사기 앱 올리고 바로 빼기"의 비용을 올린다.
- 감사 교훈: 보증금 보관은 에스크로다. `contracts-audit-2026-10-05.md` F-01(재진입 에스크로 지급불능)과 같은 계열 버그를 피하도록 checks-effects-interactions, pull 방식 출금, 보존(conservation) 퍼즈 테스트 필수.

**업데이트 지연(악성 업데이트 방어):**
- 첫 게시는 즉시. 이후 `release`는 `activatesAt = now + 48h`. 지연 동안 지갑은 이전 번들을 계속 쓰고, "새 버전 대기 중"과 diff 요약(선언 컨트랙트·권한 변화)을 보여 준다.
- 권한·선언 컨트랙트가 **늘어나는** 업데이트는 사용자 재동의 필요(MetaMask Snaps가 버전마다 재등재를 요구하는 이유와 같다 [M1]).
- 게시자 키 탈취 시 진짜 게시자는 48시간 안에 `cancelRelease`할 수 있어야 하므로, 게시자를 `EastSeaAccount`(Secure Enclave 키 + 오너 정책)로 쓰도록 권장하고, 선택적으로 "취소 전용 보조 키"를 등록.
- 긴급 보안 수정이 지연 때문에 늦어지는 문제는 "권한이 줄어드는 업데이트는 지연 1h"로 완화.

### 4.3 Manifest (오프체인 JSON, 해시는 온체인)

Farcaster manifest [F1]와 Apple 4.7.4 색인 요구 [A1]를 함께 만족하도록:

```jsonc
{
  "schema": "eastsea.app/1",
  "appId": "0x…",                    // 레지스트리 appId
  "version": "1.4.0",
  "name": "Tide Pay",                 // 32자
  "subtitle": "…",                     // 30자
  "description": "…",                  // 170자
  "category": "payments",             // 고정 목록(아래)
  "tags": ["tips","split"],          // 최대 5
  "locales": ["ko","en"],
  "icon": "icon.png",                 // 번들 안 경로(외부 URL 금지 → 해시로 고정)
  "screenshots": ["s1.png","s2.png"],
  "entry": "index.html",
  "bundle": { "sha256": "…", "size": 812345, "format": "tar-deterministic" },
  "mirrors": ["https://cdn.example/app/<sha256>.tar", "iroh-blob:<hash>"],
  "name_binding": "tidepay",          // EastSeaNames 이름(선택, 4.5 참고)
  "contracts": [                       // 이 앱이 부를 컨트랙트 전부
    {"address":"0x…","label":"Router","source":"https://…","brake":"0x…|none"}
  ],
  "connect": ["https://api.tidepay.example"],  // CSP connect-src 허용 목록
  "permissions": ["accounts","send","sign-typed","notifications"],
  "payees": ["0x…"],                  // 에이전트가 결제할 수 있는 수취 주소(4.10)
  "agent": { "summary": "…", "actions": [ … ] },   // 선택, 4.10
  "age_rating": "4+|12+|17+",         // Apple 4.7.5
  "source_repo": "https://github.com/…",
  "repro": { "builder": "…", "commit": "…" },      // 재현 빌드 배지용
  "support": "mailto:…",
  "privacy": "https://…"
}
```

**카테고리(고정):** payments, names, social, games, tools, data(익스플로러류), nft-media, defi-trading, token-issuance, other. `defi-trading`·`token-issuance`는 4.8·5의 법·정책 필터 대상.

### 4.4 지갑이 번들을 받고, 검증하고, 격리하는 방법

1. **받기:** 노드가 레지스트리 이벤트를 인덱싱(새 모듈, 예 `crates/node/src/apps_index.rs`) → 지갑이 manifest를 `mirrors` 중 아무 곳에서나 받음(https CDN, 게시자 서버, **iroh blob로 다른 EastSea 노드에서**). 출처는 신뢰하지 않는다.
2. **검증:** `sha256(manifest) == manifestHash`, `sha256(bundle) == bundleHash`, `activatesAt <= now`. 블록 확정은 로컬 노드가 이미 검증(라이트 클라이언트 경로 `crates/light`). 하나라도 틀리면 실행 거부. 이 경로는 eth.casa가 서비스 워커로 하려는 일 [E2]을 지갑 바이너리 안에서 하는 것이다.
3. **캐시와 시딩:** 검증된 번들은 로컬 콘텐츠 주소 저장소에 보관(LRU, 상한). 사용자가 켜면 iroh blob로 다른 노드에 시딩 — "노드가 앱 CDN도 된다".
4. **격리 실행:** `eastsea-app://<appId>/…` 사설 스킴(현재 `BundledPageScheme`의 일반화). 규칙:
   - CSP: `default-src 'self'; connect-src 'self' <manifest.connect>; frame-ancestors 'none'; form-action 'none'`, 원격 스크립트 금지(번들 밖 코드 실행 차단 → 해시 보장이 의미를 갖는다).
   - 권한·저장소의 origin은 **도메인이 아니라 appId**. 앱이 도메인을 바꿔도 권한이 따라가고, 같은 도메인을 사칭한 다른 앱은 권한을 못 얻는다.
   - EIP-1193 provider는 페이지 세계가 아닌 별도 `WKContentWorld`에서 브리지(페이지 JS가 prototype을 오염시켜 브리지를 속이지 못하게) [WK1].
   - **선언 컨트랙트 검사:** `eth_sendTransaction`/서명 대상이 `manifest.contracts`에 없으면 강한 경고("이 앱이 미리 밝히지 않은 주소입니다"). 승인(approve/permit/setApprovalForAll/EIP-7702 위임)은 기본적으로 한도·대상 명시 화면. Scam Sniffer 2025의 대형 피해 유형이 정확히 이것들이다 [SS1].
   - **로컬 시뮬레이션:** 서명 전에 로컬 노드에서 실행해 잔액 변화 표시(Rabby식 [R1]). 우리는 외부 API 없이 가능.
5. **일반 웹(https) dApp도 계속 연다.** 다만 레지스트리 앱이 아닌 웹 origin에는 "등록되지 않은 사이트" 표시와 현재 `BrowserOriginPolicy`의 닮은꼴 경고를 유지.

### 4.5 이름 → 앱 매핑

- **양방향 결합(Farcaster `accountAssociation`의 온체인판 [F1]):** 이름 소유자가 `EastSeaNames.setText(name, "app", <appId hex 66자>)`(값 128자 한도 안), 그리고 manifest의 `name_binding`이 같은 이름. **둘 다 맞을 때만** 지갑이 앱 이름 옆에 `@tidepay`를 표시하고, 주소창에 `tidepay`를 치면 바로 연다.
- 한쪽만 있으면 이름을 표시하지 않는다(이름 사칭 방지). 이름이 만료되면(1년+30일 유예) 결합도 자동으로 풀린다(`_sweep`가 텍스트를 지움).
- 이름 서비스는 텍스트 키가 4개뿐이므로 `app` 하나만 쓰고 나머지는 사용자 몫으로 남긴다. 컨트랙트 변경 없이 가능.
- 닮은꼴 이름 경고: `BrowserOriginPolicy`의 편집거리·혼동 문자 검사를 이름과 앱 이름에도 적용(추천 칸·검색 상위 앱 대비).

### 4.6 기기 안 검색과 Sybil에 강한 순위

**검색:** 노드가 인덱싱한 manifest(이름·부제·설명·태그·카테고리)로 로컬 전문 검색. 서버 질의 없음 → 사용자의 관심사가 밖으로 새지 않는다.

**순위 신호(기본 규칙, 공개·결정적·오픈소스):**

| 신호 | 왜 Sybil에 강한가 | 가중 |
|---|---|---|
| **등록 노드 사용자 수**: 지난 30일 앱의 선언 컨트랙트와 상호작용한 주소 중 `CommitteeRegistry`에 등록된 운영자(또는 그 노드의 지갑) 수 | 등록은 DeviceCheck로 **기기 1대 = 1키** (`14-registration.md`). World App이 World ID 사용자 수로 보상하는 것과 같은 발상 [W4] | 높음 |
| 재방문(서로 다른 7일 중 3일 이상 사용한 주소 수) | 일회성 봇 파밍보다 비용이 큼 | 중간 |
| 사용자가 낸 수수료의 **소각분** 합(계정당 상한) | 비용이 실제로 소멸 → 가짜로 부풀리면 그만큼 태워야 함 | 중간, 계정당 상한 |
| 내 그래프: 내가 돈을 보낸 주소들이 쓰는 앱(개인화 EigenTrust) | 나와 관계없는 봇 클러스터는 내 순위에 영향이 거의 없음 [F7] | 개인화 칸에서만 |
| 앱 나이·업데이트 지연 준수 이력, 플래그 없음 | 급조 사기 앱 불리 | 보조 |

**쓰지 않는 신호:** 거래량, 단순 고유 주소 수, 설치 수, 별점(기기 밖 리뷰 서버가 없으면 조작 검증 불가). 거래량은 워시 트레이딩 역사 [D2]가 보여 주듯 가장 싸게 조작된다.

**조작 방어 규칙:** 게시자 주소와 그 자금 출처(첫 입금 주소) 클러스터의 상호작용 제외 / 신규 주소(첫 거래 7일 미만) 제외 / 주소당 기여 상한 / 7일 이동창(Base 방식 [B1]) / 계산식·버전을 화면에 공개.

**교체 가능한 순위("피드"):** Bluesky 피드 생성기처럼 [BS2], 순위는 "앱 ID 목록을 돌려주는 부품". 기본 피드(위 규칙, 지갑 내장) 외에 사용자가 **서명된 목록**(게시자 키가 서명한 JSON, 해시를 레지스트리 옆 작은 `Lists` 컨트랙트나 이름 텍스트로 공지)을 구독할 수 있다. Nostr NIP-89처럼 "내가 팔로우한 사람의 추천"도 같은 구조로 표현 가능 [N1]. 지갑은 어떤 피드를 보고 있는지 항상 표시.

### 4.7 배지 (사실만, 보증 아님)

| 배지 | 조건(기계 확인) | 뜻하지 않는 것 |
|---|---|---|
| **이름 확인** `@name` | 4.5의 양방향 결합 | 게시자가 선량함 |
| **소스 일치** | `repro`의 커밋을 재현 빌드했을 때 bundleHash 일치. 확인자는 누구나, 결과는 증명 서명으로 게시. 지갑은 사용자가 신뢰하는 확인자 목록으로 판단 (`docs/research/repro-builds-2026.md`) | 소스가 안전함 |
| **컨트랙트 소스 공개** | 선언 컨트랙트 전부의 런타임 바이트코드가 공개 소스 빌드와 일치 | 감사됨 |
| **Brake 있음** | 선언 컨트랙트가 공개 brake 인터페이스(예: `brakeState()`, 가디언·타임락·자금 탈출 경로 문서 링크)를 구현하고, 현재 상태를 로컬 `eth_call`로 확인 | brake 가디언이 정직함 |
| **감사 증명** | 감사인 키가 서명한 증명(대상 바이트코드 해시, 보고서 해시, 날짜). 감사인 목록은 사용자가 고름. MetaMask Snaps처럼 위험 권한(키 관리 대응: `sign-typed`의 무제한 승인, 위임)을 요구하는 앱은 감사 증명 없으면 **설치 전 추가 확인 단계** [M1] | 버그 없음 |
| **Brake 없음 / 감사 없음** | 위 조건 불충족 시 **중립 회색 표기**로 반드시 표시 | — |

"추천" 칸과 기본 순위 상위 노출의 조건에 `defi-trading`, `token-issuance` 카테고리는 **"Brake 있음 + 감사 증명"**을 요구한다. 이것이 genesis 원칙(brake 전 DEX·런치패드 배포 금지)을 제3자에게 강요하지 않으면서, 우리 지갑이 brake 없는 금융 앱을 **밀어주지는 않는** 방법이다.

### 4.8 신고·플래그·차단: 중앙 관리자 없이 (또는 최소·투명하게)

**층 1 — 서명 시점(항상 켜짐, 목록 불필요):** 선언 컨트랙트 검사, 무제한 승인·Permit·EIP-7702 위임 강조, 로컬 시뮬레이션. 손실의 대부분을 잡는 층이다 [SS1][M2].

**층 2 — 쌓을 수 있는 플래그 목록(labeler):**
- 플래그 = `(appId 또는 bundleHash 또는 컨트랙트 주소, 라벨, 증거 해시, 서명자)`. 라벨: `phishing`, `drainer`, `impersonation`, `malware`, `illegal-in-KR`, `spam`, `broken`.
- 플래그 게시는 누구나(작은 `Flags.sol`에 보증금 + 이벤트, 또는 서명된 오프체인 목록의 해시 공지). Bluesky labeler [BS1] + Phantom 공개 blocklist [P1]의 결합.
- 지갑 기본 구독 목록 1개: **"EastSea 안전 목록"** — 공개 멀티시그(예: 3/5, 구성원 공개), **모든 추가·삭제를 투명성 로그(온체인 이벤트)로 남기고 증거 링크 필수, 이의 제기 경로 공개**. 이것이 "최소·투명한 관리자"다. 사용자는 끄거나 다른 목록을 더할 수 있다.
- 효과는 **표시층에서만**: `phishing/drainer/malware` → 기본 차단 화면(사용자가 이유를 읽고 "그래도 열기" 가능, 단 서명은 추가 확인), 나머지 → 경고 띠. 레지스트리 등재 자체는 아무도 지우지 못한다(ar.io의 "게이트웨이별 자율 차단" [AR2]와 같은 철학).
- 번들 해시 단위 플래그: 같은 앱의 다른 버전은 영향 없음 → 오탐 피해 축소. 악성 업데이트는 해당 bundleHash만 막고 이전 버전으로 되돌릴 수 있다.

**층 3 — 오프라인 대비:** 기본 안전 목록의 스냅샷을 지갑 업데이트에 동봉(`ReleaseLog.sol` 경로와 같은 릴리스 승인), 노드 동기화 시 최신화. Phantom이 목록을 로컬에 두고 주기 갱신하는 방식 [P1].

**하지 않는 것:** 토큰 투표로 목록 관리(TCR 실패 [TC1]), 보증금 몰수, 레지스트리 관리자 키.

### 4.9 Apple: iOS 앱 안에서 허용되는 것

App Review Guidelines 원문 [A1] 기준 정리:

| 조항 | 요지 | 우리 설계에 미치는 것 |
|---|---|---|
| 2.5.2 | 앱 기능을 바꾸는 코드 다운로드·실행 금지 | 4.7 예외(HTML5/JS 미니앱)로만 가능. **네이티브 코드·WASM 플러그인으로 지갑 기능을 바꾸면 안 됨** |
| 2.5.6 | 웹 브라우징은 WebKit 사용 | WKWebView 그대로 |
| 4.7 | HTML5·JS 미니앱, 챗봇, 플러그인 허용 | 레지스트리 앱 = HTML5 번들이므로 범위 안 |
| 4.7.1 | 개인정보 지침, **콘텐츠 필터링과 신고 기능**, 3.1(결제) 준수 | 4.8 플래그 + 앱별 "신고" 버튼 필수 |
| 4.7.2 | 네이티브 플랫폼 API를 Apple 허가 없이 노출 금지 | 카메라·위치·연락처 등을 미니앱에 넘기지 않는다. 지갑 provider는 우리 기능이지만 **해석 위험**이 있어 심사 노트에 명시 |
| 4.7.3 | 데이터·권한 공유는 **매번 명시적 동의** | 계정 연결·서명 매번 시트(이미 그러함) |
| 4.7.4 | **제공하는 소프트웨어 전체의 색인 + universal link** | `eastsea.xyz/app/<appId>` 정적 색인 페이지와 universal link(5.2와 겸용) |
| 4.7.5 | 연령 부적합 콘텐츠 식별·제한 | manifest `age_rating` + iOS 기본 필터 |
| 3.1.1 | 디지털 기능 잠금 해제는 IAP. NFT 관련 서비스는 IAP로 판매 가능, **NFT 소유가 기능을 잠금 해제하면 안 됨** | iOS에서 "코인으로 디지털 아이템 구매" 미니앱은 위험. 송금·결제(실물·P2P)와 구분 |
| 3.1.5(i) | 지갑은 **조직 계정 개발자**만 | Pipln 조직 계정 필요 |
| 3.1.5(iii) | 거래소 기능은 면허 있는 국가·지역에서만 | iOS에서 `defi-trading` 카테고리 기본 숨김(또는 지역 제한) |
| 3.1.5(iv) | ICO·선물·준증권 거래는 인가 금융기관만 | iOS에서 `token-issuance` 카테고리 숨김 |
| 3.1.5(v) | **다운로드 권유·SNS 게시 등 과제 완료에 코인 지급 금지** | 코인 리퍼럴·과제 보상 전면 금지(5.6) |

추가 사실:
- Apple **Mini Apps Partner Program**(2025-11): 제3자 미니앱의 IAP 수수료 15%, 조건은 4.7 준수, **승인된 manifest**(호스팅 미니앱과 메타데이터), Advanced Commerce API, Declared Age Range API [A2][A4]. 우리는 IAP를 안 쓰므로 가입 대상은 아니지만, **Apple이 "제3자 미니앱 디렉터리를 가진 호스트 앱"을 공식 범주로 인정**했다는 점이 심사 설명에 유리하다.
- 선례 위험: Coinbase Wallet은 2019-12 iOS에서 dApp 브라우저를 Apple 정책 때문에 제거했다 [A3]. 이후 지침이 4.7로 바뀌었지만, **심사관 재량 위험은 남는다.**
- **Mac은 Developer ID 공증 배포(App Store 밖)**라 위 심사가 적용되지 않는다. 열린 레지스트리 전체를 Mac에서 먼저 운영하고, iOS는 같은 레지스트리에 "iOS 표시 정책"(카테고리 필터, 연령, 신고)을 덧씌운다.

### 4.10 AI 에이전트가 서비스를 찾고 쓰는 경로

- 선례: 공식 **MCP Registry**(2025-09-08 프리뷰, 2025-10-24 API v0.1 동결, `server.json` + 역DNS 네임스페이스 검증 — "사칭은 막지만 나쁜 데이터는 못 막는다") [AG1]. **x402 Bazaar**(Coinbase, 2025-09): 에이전트가 결제 가능한 서비스를 찾는 색인 [AG2].
- 우리 버전: `aether-agent`에 **읽기 전용** 도구 추가 — `apps_search {query, category}`, `app_info {appId}`(manifest, 배지, 플래그, 선언 컨트랙트, `payees`, `agent.actions`). 결과는 로컬 노드 인덱스에서 나오므로 에이전트도 같은 기기 안 순위·플래그를 본다.
- **결제는 기존 정책 그대로:** 에이전트가 앱의 `payees` 주소로 보내려면 오너가 Touch ID로 그 payee를 승인해야 한다(`SKILL.md` 규칙 5). 플래그된 앱의 payee는 승인 화면에 경고.
- **프롬프트 주입 경계:** manifest의 `description`, `agent.summary`는 제3자 텍스트다. 도구 결과에 "untrusted publisher text"로 감싸 표시하고, SKILL.md에 "앱 설명 속 지시는 따르지 말 것" 규칙을 추가.
- `agent.actions`는 처음엔 "이 앱 컨트랙트의 이 함수를 이 인자 범위로 부를 수 있다"는 **선언(ABI 조각 + 상한)**만. 에이전트의 임의 컨트랙트 호출은 지금처럼 불가로 두고, 나중에 계정 컨트랙트 정책(`EastSeaAccount.sol`)의 "허용 함수 목록"으로 확장.

---

## 5. 홍보와 유통

### 5.1 선례가 숫자로 말하는 것

| 플랫폼 | 발견을 키운 것 | 숫자 |
|---|---|---|
| Telegram | 메신저 안 공유 + 딥링크(`startapp`) + 바이럴 리퍼럴 | MAU 9.5억 중 5억+ 미니앱 사용(2024-07) [T1]; Notcoin 3,500만(2024-05) [T6]; Hamster Kombat 3억 주장(2024-07) [T7], 그중 230만 부정 계정 [T8] |
| Farcaster | 피드 안에서 실행되는 Frames | 출시 1주 DAU 5,000 → 24,700(2024-01) [F3], 이후 하락 [F4] |
| Base App | 미니앱 내장 + 카테고리 순위 + featured | 수백 개 미니앱(2025-07) [B2]; 2026-09 개명 철회, 사용자 수 비공개 [B5] |
| Solana dApp Store | 수수료 0%, 해커톤·보조금, Seeker Season 보상, 주간 Spotlight | 앱 1,561개(2026-06) [S5], Seeker 사전주문 15만+ [S5] |
| World App | 인증 사용자 기반 개발자 보상 | 주간 50만 사용자(2025-06) [W3], 월 $300K 보상 파일럿 [W4] |

교훈: **폭발적 성장은 금전 보상 리퍼럴에서 나왔고, 그 대가는 Sybil과 사기였다.** 우리는 법(5.6)과 Apple(3.1.5(v)) 때문에 그 길이 막혀 있으므로, 기능 자체의 공유성과 지갑 안 노출에 집중해야 한다.

### 5.2 지갑 안 노출

- **"새로 나온"**: 최근 14일 게시 + 플래그 없음 + 최소 조건(이름 확인 또는 등록 노드 사용자 N명) 앱을 시간순(순위 조작 여지 최소).
- **"추천"**: 주 1회 4개 내외(Solana Spotlight 방식 [S6]), 라벨 "EastSea 팀 추천", 선정 기준 공개(Base featured처럼 로드 3초·외부 리다이렉트 없음·가스 대납 등 품질 기준 [B4]), 우리 앱은 추천 칸에 올리지 않거나 "자사" 표기. `defi-trading`·`token-issuance` 제외(5.6).
- **"내 주변에서 쓰는"**: 내가 송금한 주소들이 쓰는 앱(개인화 피드, 기기 안 계산).

### 5.3 딥링크·universal link·QR

- `https://eastsea.xyz/app/<appId>` (또는 `/n/<name>`): universal link로 지갑이 설치돼 있으면 지갑 안 Explore에서 바로 열고, 없으면 정적 랜딩(5.4). Telegram `startapp` 매개변수처럼 `?start=<payload>`를 앱에 전달 [T3]. 이 경로가 Apple 4.7.4의 "universal link 색인"도 겸한다 [A1].
- QR은 같은 URL. **지갑이 QR을 열 때 이름 결합·플래그 상태를 먼저 보여 준다**(오프라인 QR 피싱 대비).
- `eastsea://` 커스텀 스킴은 다른 앱이 가로챌 수 있으므로 보조 수단으로만.

### 5.4 서비스별 정적 랜딩 페이지와 SEO

- 레지스트리 이벤트로 `eastsea.xyz/app/<appId>` 정적 페이지를 **빌드 시 생성**(manifest 이름·설명·스크린샷·배지·선언 컨트랙트·플래그 상태). 서버 없이 R2/CDN에 올림(`public-read-access` 문서의 정적 이력 방향과 같은 인프라).
- 이 페이지는 **표시만** 한다(지갑 연결·서명 없음) → 법률 메모의 "Pipln 도메인에서 거래 UI 직접 호스팅" 위험(3.7)을 피함. 거래·발행 카테고리는 정적 페이지에서도 "지갑 안에서만 열림" + 위험 고지.
- `noindex` 필드(Farcaster/Base와 같은 의미 [F1][B1])를 존중, 플래그된 앱은 `noindex` 강제.

### 5.5 런칭 커뮤니티·소셜·빌더

- **조코헌트(JocoHunt):** 주간 순수 득표순, GitHub 7일+커밋 1회 계정만 투표, 자기 투표 불인정, 부계정·표 교환은 운영자 검토 대상. 41주차 첫날 1위가 1표일 만큼 초기 (`docs/launch/jocohunt.md`, 2026-10-04 관찰). 우리 플랫폼 출시와 별개로, **레지스트리에 올라온 제3자 서비스들이 조코헌트에 출시하도록 안내**("EastSea 지갑에서 열기" 버튼 = universal link)하면 생태계 노출이 함께 쌓인다.
- **Product Hunt:** 2025년 이후 featured 비율이 크게 줄었다는 2차 보고(2023-09 하루 47개 → 2024-09 16개) [PH1]. 개발자 도구 카테고리로 지갑+레지스트리 CLI를 소개하는 편이 맞다. 코인 언급 금지 원칙 동일.
- **Show HN:** `docs/launch/demo-and-show-hn.md` 계획과 연계 — "모든 사용자가 노드인 지갑에서 앱 번들을 해시로 검증해 실행" 자체가 HN에 맞는 기술 이야기.
- **빌더 템플릿·해커톤:** Base의 `npx create-onchain --mini` [B3], Solana의 해커톤·보조금 [S5]처럼 `npm create eastsea-app` 템플릿 3종(결제 버튼, 이름 기반 프로필, 읽기 전용 대시보드). 해커톤 상금은 **법정화폐·물품**으로(코인 상금은 법률 검토 전 보류).

### 5.6 리퍼럴 메커니즘과 한국 법적 위험

- **Apple 3.1.5(v):** "다운로드 권유, SNS 게시 등 과제 완료에 코인 지급" 금지 [A1] → iOS 앱에서 코인 리퍼럴은 바로 리젝 사유.
- **금융위원회(2025-12):** "국내 미신고 가상자산 취급업자를 블로그·SNS로 홍보·알선(레퍼럴)하는 행위"를 미신고 가상자산사업 유형으로 봄. 근거는 가상자산이용자보호법·특금법상 "중개·알선" 정의 [K1](헤럴드경제 2026-01-04 보도, 원문 공지는 미확인 — 변호사 확인 필요).
- **유사수신·방문판매법(다단계):** 신규 회원 모집 수당 구조는 사기·유사수신·방문판매법 위반으로 수사된 선례가 있다 [K2].
- **법률 메모 3.6** (`docs/research/legal-opinion-memo-2026-10-04.md`): 가격·예상 수익률·상장 언급 금지, "먼저 온 사람이 더 큰 몫" 같은 표현 금지. 3.8: 런치패드는 랭킹·트렌딩 배제가 조건.
- **결론:** (a) 코인·포인트·수수료 환급 형태의 리퍼럴 보상 **없음**. (b) 허용: "초대한 사람" 표시, 공유 링크 클릭 수를 게시자에게만 보여 주는 익명 통계, 비금전 배지. (c) 지갑의 "추천"·순위·트렌딩에서 `defi-trading`·`token-issuance` 제외 — 우리 지갑이 특정 거래·발행 서비스를 노출·추천하면 "알선"으로 해석될 위험. (d) 제3자 앱 내부의 리퍼럴은 그 게시자의 책임이지만, 플래그 라벨 `illegal-in-KR`로 한국 지역 경고를 붙일 수 있게 한다.

---

## 6. 개발자 경험(DX): 제3자가 첫날 출시하려면

### 6.1 선례의 온보딩 방식

| 플랫폼 | 도구 | 마찰 | 악성 앱 대응(게이트키퍼 없이 또는 있이) |
|---|---|---|---|
| Solana dApp Store | Publisher Portal, `dapp-store` CLI, NFT 자동 발행, ~0.2 SOL [S2][S4] | KYC/KYB, 3~5일 심사 | 중앙 심사 + 신고 메일 + 재량 삭제 [S3] |
| Telegram | BotFather, Mini Apps SDK, initData 서명 검증 [T3] | 낮음(카탈로그 노출만 조건) | 사후 차단 위주, FEMITBOT 같은 대규모 악용 [T5] |
| Farcaster | manifest 도구, Neynar 템플릿·API [F1][F6] | 매우 낮음 | 심사 없음. 외부 평판 점수(Neynar score), OpenRank [F6][F7], 사용량 기준 색인 [F2] |
| Base | `create-onchain --mini`, Base Build 대시보드, Paymaster [B3][B4] | 낮음(색인), featured는 신청 | 색인은 공유+사용량, featured는 품질 심사 [B1][B4] |
| MetaMask Snaps | npm 게시, 위험 권한만 감사·allowlist [M1] | 권한별 | open 권한은 permissionless, 위험 권한은 감사 필수 |

### 6.2 우리가 제공할 것 (우선순위 순)

1. **Provider·SDK 문서:** 확장과 인앱 브라우저가 같은 EIP-1193 메서드 집합(`methods.js` ↔ `BrowserPolicy.swift`)을 쓰므로 한 문서로. `aether_getAccount`(증명 동봉), `aether_accountHistory`, 이름 해석, 선언 컨트랙트 규칙, CSP 제약(원격 스크립트 불가)을 명시.
2. **템플릿:** `npm create eastsea-app` — Vite 정적 번들, manifest 생성, 결정적 tar 빌드 스크립트 포함.
3. **게시 CLI `eastsea-app`:** `build`(결정적 번들, sha256) → `manifest`(검증·해시) → `publish`/`release`(레지스트리 트랜잭션; 서명은 지갑/Secure Enclave에 위임, 개인키 파일 없음) → `mirror`(iroh blob 시딩 + 선택 CDN 업로드) → `verify`(남의 앱 재현 빌드 확인, 소스 일치 증명 게시). Solana CLI처럼 한 줄 업데이트 [S4], 그러나 KYC·포털 없음.
4. **테스트넷 수도꼭지와 로컬 devnet:** 이미 `aether_get_test_tokens`, `crates/node/src/faucet.rs`가 있다. `eastsea-app dev`가 로컬 노드에 레지스트리를 배포하고 지갑의 "개발자 모드"(로컬 번들 경로를 해시 검증 없이 열되 빨간 띠 표시)로 즉시 열기.
5. **배지 경로 문서:** 이름 결합 → 소스 일치(재현 빌드) → 컨트랙트 소스 공개 → brake 인터페이스 → 감사 증명. 각 단계가 기계로 확인되는 방법과 "배지가 뜻하지 않는 것".
6. **추적 없는 분석:** 게시자에게 주는 통계는 **체인에서 누구나 계산할 수 있는 것만**(선언 컨트랙트 상호작용 주소 수, 등록 노드 사용자 수, 재방문 분포, 7일 창). 지갑은 앱 열람·검색 기록을 밖으로 보내지 않는다. 필요하면 나중에 지갑이 국소적 차등 프라이버시로 "열린 횟수" 근사치를 집계하는 선택 기능.
7. **미니앱 안전 가이드:** 승인 최소화, Permit 만료 짧게, EIP-7702 위임 요구 금지 권고, 선언 컨트랙트 밖 호출 금지.

---

## 7. 단계별 계획 (가장 작은 구체 단계와 저장소 영역)

### 단계 0 — 이번 주: 명세와 기반 (코드 영향 최소)
1. `docs/design/31-app-registry.md`: 4.2 컨트랙트 인터페이스, 4.3 manifest 스키마(JSON Schema 파일 `docs/design/schemas/eastsea-app-1.json`), 4.4 검증·격리 규칙, 4.6 순위 식, 4.8 플래그 형식, Apple 4.7 대응표.
2. 법률 질의 목록(변호사): 레지스트리 운영·추천 칸·정적 랜딩 페이지가 "중개·알선"에 해당하는지, 거래 카테고리 표시 기준, 해커톤 상금 형태.

### 단계 1 — 베타(테스트넷): 1st-party 앱을 "레지스트리 방식"으로, 동시에 제3자 게시 개방
1. **`BundledPageScheme` 일반화** (`apps/wallet/Sources/BundledPageScheme.swift`, `BrowserOriginPolicy.swift`, `SitePermissions.swift`): `eastsea-app://<appId>/` + 해시 검증 + appId 단위 권한 + manifest 기반 CSP. 익스플로러(`apps/explorer`)를 첫 번째 manifest 앱으로 이식.
2. **`contracts/src/AppRegistry.sol` + 테스트**(`contracts/test/`): publish/release/지연/취소/unlist/보증금 출금, 보존 퍼즈, 재진입 테스트. 이름 서비스와 같은 2단계 이전.
3. **노드 인덱서** (`crates/node/src/` 새 모듈 + `rpc.rs`에 `aether_listApps`, `aether_getApp`): 이벤트 인덱싱, manifest 캐시, 순위 신호 계산(계정 이력 인덱스·`CommitteeRegistry` 조회 재사용).
4. **게시 CLI + 템플릿**(`scripts/` 또는 `apps/cli`): `build/manifest/publish/release/mirror/verify/dev`.
5. **이름 앱**(1st-party): 이름 등록·`app` 텍스트 레코드 설정 UI를 레지스트리 앱으로 게시.
6. **tx 시트 선언 컨트랙트 검사** (`BrowserPolicy.swift`, `apps/extension/src/lib/methods.js` 동시 — 기존 동등성 테스트에 케이스 추가).
7. **기본 안전 목록 v0**: 멀티시그 미정이면 우리 키 1개 + 투명성 로그 + "베타 한정" 명시. 단계 2 전에 멀티시그로 교체.
- 이 단계에서 **제3자도 테스트넷 레지스트리에 바로 게시 가능**(보증금은 테스트 코인). 창업자 정정 반영.

### 단계 2 — 메인넷: 열린 레지스트리
1. `AppRegistry.sol` 외부 감사(작은 범위) 후 genesis predeploy 또는 출시 직후 배포(불변, 관리자 없음). 런타임 해시를 클라이언트에 고정(`contracts-audit` 권고 3과 같은 방식).
2. `Flags.sol`(또는 서명 목록 공지) + 지갑 labeler 구독 UI, 기본 안전 목록 멀티시그 전환.
3. 배지: 이름 결합, 컨트랙트 소스 일치, brake 인터페이스 확인(로컬 `eth_call`).
4. 정적 랜딩 페이지 생성기(`site/` 빌드 단계) + universal link(`apple-app-site-association`, `apps/wallet/project.yml` associated domains).
5. 에이전트 읽기 도구 `apps_search`, `app_info` (`apps/agent/Sources/Tools.swift`, `MCP.swift`, `agents/skills/aether-wallet/SKILL.md` 규칙 추가).
6. "새로 나온" 칸, 기본 순위 피드.

### 단계 3 — 확장
1. iOS: 4.7 의무(신고 버튼, 연령, 색인 페이지, 매번 동의) 구현 후 iOS 표시 정책(거래·발행 숨김)으로 제출. 심사 노트에 Mini Apps Partner Program과 4.7 근거 명시.
2. 교체 가능한 피드(서명 목록 구독), 개인화(내 송금 그래프) 피드.
3. 재현 빌드 확인자 네트워크, 감사 증명 형식.
4. iroh blob 번들 시딩 기본값 검토(저장소 보상 연구 `codex-storage-reward` 워크트리와 연결 가능).
5. receipts/logs 커밋이 체인에 들어오면(`public-read-access` §0-5) 순위 신호를 "검증된 데이터"로 승격.

---

## 8. 위험 (명시)

| # | 위험 | 심각도 | 완화 | 남는 위험 |
|---|---|---|---|---|
| 1 | **지갑 안 미니앱 피싱**: 지갑 UI 안에서 열리므로 사용자가 더 믿음(FEMITBOT [T5]) | 높음 | 앱 화면에 항상 지갑 소유 띠(앱 이름·배지·플래그), 서명 시점 검사·시뮬레이션, 닮은꼴 이름 경고 | 신규 사기 앱은 플래그 전 몇 시간 노출 |
| 2 | **게시자 키 탈취 → 악성 업데이트** | 높음 | 48h 업데이트 지연, 권한 증가 시 재동의, 해시 단위 플래그, Secure Enclave 게시자 권장 | 게시자가 48h 안에 알아채지 못하면 실행됨 |
| 3 | **기본 안전 목록 = 사실상 중앙 검열자**가 될 위험 | 중간 | 표시층 효과만, 투명성 로그·증거 필수·이의 제기, 사용자가 끄고 대체 가능 | 대부분 기본값을 유지하므로 영향력은 큼 |
| 4 | **순위 조작**(등록 노드도 여러 대 Mac으로 늘릴 수 있음) | 중간 | 계정당 상한, 게시자 클러스터 제외, 소각 비용, 7일 창, 신호 공개 | 공개 식은 공격자도 읽음 → 정기 개정 필요 |
| 5 | **한국 법: 추천·노출이 중개·알선으로 해석** [K1] | 높음 | 거래·발행 카테고리 추천·트렌딩 제외, 정적 페이지는 표시만, 변호사 검토 | 레지스트리 운영 자체에 대한 해석 미확정 |
| 6 | **Apple 리젝**(4.7.2 해석, 3.1.5, 2019 Coinbase 선례 [A3]) | 중간 | iOS 보수적 표시 정책, 4.7 의무 선구현, Mac 우선 | 심사관 재량 |
| 7 | **보증금 에스크로 버그** | 중간 | 감사, pull 출금, 보존 퍼즈(F-01 교훈) | — |
| 8 | **번들 가용성**: 게시자 서버가 내려가면 앱 사라짐 | 낮음~중간 | iroh 시딩, 여러 미러, 로컬 캐시 | 아무도 시딩 안 한 신규 앱 |
| 9 | **이벤트 데이터 미검증**: receipts 커밋 전까지 순위 신호는 "내 노드가 실행한 결과"라 자기 노드에선 신뢰 가능하지만 라이트 클라이언트·웹 랜딩에서는 미검증 | 낮음 | 지갑 노드에서만 계산, 웹 페이지엔 "노드 계산" 표기 | — |
| 10 | **에이전트 프롬프트 주입**(manifest 텍스트) | 중간 | 도구 결과 untrusted 표기, SKILL 규칙, 결제는 Touch ID payee 승인만 | 읽기 단계 오도(잘못된 추천) |
| 11 | **소셜 피드 의존의 실패**(Farcaster·Base [F5][B5]) | 전략 | 피드 대신 지갑 고유 신호(송금 그래프, 이름, 노드) 사용 | 초기 콜드스타트 — 템플릿·해커톤·조코헌트로 보완 |
| 12 | 카탈로그가 커지면 저품질 범람(Solana 1,500개 시점 [S5][S6]) | 낮음 | "새로 나온" 최소 조건, 추천 칸, 피드 교체 | — |

---

## 9. 출처 (확인일 2026-10-05, 괄호 안은 게시일)

- [S1] Solana Mobile, dApp Store 개요: https://docs.solanamobile.com/solana-mobile-stack/dapp-store (문서, 확인 2026-10-05)
- [S2] Solana Mobile, Submit new app: https://docs.solanamobile.com/dapp-store/submit-new-app.md (확인 2026-10-05)
- [S3] Solana Mobile Publisher Policy: https://legal.solanamobile.com/publisher-policy-web (확인 2026-10-05)
- [S4] Solana Mobile, Publishing CLI / App NFT: https://docs.solanamobile.com/dapp-store/publishing-cli , https://docs.solanamobile.com/dapp-store/publishing-cli/app-nft (확인 2026-10-05)
- [S5] Solana Compass, "dApp Store Surpasses 1,561 Apps": https://solanacompass.com/news/solana-mobile-dapp-store-surpasses-1561-apps-as-catalog-more-than-doubles-in-three-months (2026-06-26)
- [S6] Solana Mobile 블로그, "1,000+ dApps, Smarter Discovery…": https://solanamobile.com/blog/1-000-dapps-smarter-discovery-and-a-bigger-seeker-season (2026-07-14)
- [T1] Telegram Info, 950M MAU / 500M+ 미니앱: https://t.me/s/tginfoen?after=1942 ; 보도 https://wnhub.io/news/stores-and-publishing/item-44442 (2024-07-22~31)
- [T2] Cointelegraph, Mini App Store: https://cointelegraph.com/news/telegram-mini-app-store-end-july-pavel-durov (2024-07)
- [T3] Telegram, Mini Apps 문서: https://core.telegram.org/bots/webapps (확인 2026-10-05)
- [T4] Telegram Blockchain Guidelines: https://core.telegram.org/bots/blockchain-guidelines (2025-01, 확인 2026-10-05)
- [T5] BleepingComputer, "Telegram Mini Apps abused for crypto scams, Android malware delivery" (CTM360 FEMITBOT): https://www.bleepingcomputer.com/news/security/telegram-mini-apps-abused-for-crypto-scams-android-malware-delivery/ (2026-05-03)
- [T6] Decrypt, Notcoin: https://decrypt.co/223640/what-is-notcoin-telegram-based-game-airdrop (2024-05)
- [T7] Decrypt, Hamster Kombat 300M: https://decrypt.co/242370/telegram-game-hamster-kombat-300-million-players (2024-07)
- [T8] Decrypt, Hamster Kombat purge: https://decrypt.co/250940/why-hamster-kombat-purged-millions-players-telegram-airdrop ; The Defiant https://thedefiant.io/news/tokens/hamster-kombat-faces-backlash-for-excluding-57-of-users-from-airdrop (2024-09)
- [W1] World, Mini App Store: https://docs.world.org/mini-apps/quick-start/app-store.md (확인 2026-10-05)
- [W2] World, App Review Guidelines: https://docs.world.org/mini-apps/guidelines/policy.md (확인 2026-10-05)
- [W3] The Block, World mini apps 500K weekly: https://www.theblock.co/post/359061/worlds-mini-app-ecosystem-holds-steady-at-500000-weekly-users-amid-us-launch-and-new-partnerships (2025-06-20)
- [W4] Blockworks, Mini Apps 1.2 / Developer Rewards: https://blockworks.com/news/world-launches-mini-apps-1-2 (2025-03; 본문 접근 403, 검색 요약 기반) ; Crowdfund Insider https://www.crowdfundinsider.com/2024/12/233874-world-network-continues-to-scale-providing-mini-apps-devs-with-on-chain-verifiable-human-audience/ (2024-12)
- [F1] Farcaster Mini Apps Specification: https://miniapps.farcaster.xyz/docs/specification (확인 2026-10-05)
- [F2] Farcaster, Discovery guide: https://miniapps.farcaster.xyz/docs/guides/discovery (확인 2026-10-05)
- [F3] The Block, Farcaster DAU surge after Frames: https://www.theblock.co/post/275971/farcaster-daily-active-users-surge-frames-launch (2024-01/02)
- [F4] BlockEden, "Farcaster in 2025: the protocol paradox" (2차 출처): https://blockeden.xyz/blog/2025/10/28/farcaster-in-2025-the-protocol-paradox/ (2025-10-28)
- [F5] Neynar, "Neynar is acquiring Farcaster": https://neynar.com/blog/neynar-is-acquiring-farcaster ; The Block https://www.theblock.co/post/386549/haun-backed-neynar-acquires-farcaster-after-founders-pivot-to-wallet-app (2026-01-21)
- [F6] Neynar user quality score: https://docs.neynar.com/docs/neynar-user-quality-score ; 재학습 https://neynar.com/blog/retraining-neynar-user-score-algorithm (2025-05)
- [F7] Karma3 Labs OpenRank (EigenTrust, Farcaster API): https://decrypt.co/219892/karma3-labs-raises-a-4-5m-seed-round-led-by-galaxy-and-ideo-colab-to-build-openrank-a-decentralized-reputation-protocol (2024-03-01)
- [B1] Base, Search & discovery: https://docs.base.org/mini-apps/features/search-and-discovery (확인 2026-10-05)
- [B2] The Block, Base App 출시: https://www.theblock.co/post/362713/coinbase-unveils-base-app-rebrands-wallet-as-all-in-one-social-and-trading-platform (2025-07-16)
- [B3] Base, MiniKit quickstart: https://docs.base.org/builderkits/minikit/quickstart (확인 2026-10-05)
- [B4] Base, Featured guidelines: https://docs.base.org/mini-apps/featured-guidelines/overview , https://docs.base.org/mini-apps/get-featured/requirements (확인 2026-10-05)
- [B5] The Block, "Coinbase rebrands Base App back to Coinbase Wallet": https://theblock.co/news/defi/2026-09-10-coinbase-rebrands-base-app-back-to-coinbase-wallet-after-just-over-a-year-as-social-experiment-falls-short-414115 (2026-09-10)
- [M1] MetaMask, Get allowlisted: https://docs.metamask.io/snaps/how-to/get-allowlisted.md ; "Two Exciting Updates to MetaMask Snaps": https://metamask.io/news/two-exciting-updates-to-metamask-snaps (확인 2026-10-05)
- [M2] The Block, Consensys acquires Wallet Guard (Blockaid 기본 경고 언급): https://www.theblock.co/post/303347/consensys-acquires-wallet-guard-to-help-protect-metamask-users-against-hacks-and-scams (2024-07-03)
- [BL1] Blockaid dApp Scanning: https://blockaid.io/dapp-scanning (확인 2026-10-05, 회사 자체 수치)
- [P1] Phantom Blocklist: https://docs.phantom.app/developer-powertools/blocklist ; https://github.com/phantom/blocklist (확인 2026-10-05)
- [R1] Rabby GitHub: https://github.com/RabbyHub/Rabby (확인 2026-10-05; 시뮬레이션 설명은 2차 요약)
- [SS1] Scam Sniffer 2025 Crypto Phishing Report: https://drops.scamsniffer.io/scam-sniffer-2025-crypto-phishing-losses-fall-83-to-84-million/ (2026-01)
- [E1] eth.limo Q1 2026 Update: https://discuss.ens.domains/t/eth-limo-q1-2026-update/22082 (2026-04)
- [E2] eth.casa 소개: https://discuss.ens.domains/t/introducing-eth-casa-client-side-ens-gateway/22325 (2026-07-30)
- [AR1] ar.io, ArNS: https://docs.ar.io/learn/arns/ (확인 2026-10-05)
- [AR2] ar.io, Content Moderation: https://docs.ar.io/gateways/moderation (확인 2026-10-05)
- [N1] Nostr NIP-89: https://github.com/nostr-protocol/nips/blob/master/89.md (확인 2026-10-05)
- [N2] Nostr NIP-78: https://github.com/nostr-protocol/nips/blob/master/78.md (확인 2026-10-05)
- [BS1] Bluesky, "Stackable Approach to Moderation": https://bsky.social/about/blog/03-12-2024-stackable-moderation (2024-03-12)
- [BS2] Bluesky, getFeedSkeleton: https://docs.bsky.app/docs/api/app-bsky-feed-get-feed-skeleton ; feed-generator https://github.com/bluesky-social/feed-generator (확인 2026-10-05)
- [D1] DappRadar, "Why DappRadar is stamping down on deceptive and manipulated traffic data": https://dappradar.com/blog/why-dappradar-is-stamping-down-on-deceptive-and-manipulated-traffic-data (본문 403, 검색 요약 기반)
- [D2] Cointelegraph Magazine, "4 out of 10 NFT sales are fake" (2022 워시 트레이딩 수치): https://cointelegraph.com/magazine/4-out-of-10-nft-sales-are-fake-learn-to-spot-the-signs-of-wash-trading (2023)
- [TC1] TCR 게임이론 분석: https://arxiv.org/pdf/1809.01756 (2018) ; Gitcoin, Token Curated Registry: https://gitcoin.co/mechanisms/token-curated-registry
- [A1] Apple App Review Guidelines(2.5.2, 2.5.6, 3.1.1, 3.1.5, 4.7): https://developer.apple.com/app-store/review/guidelines/ (확인 2026-10-05)
- [A2] Apple, Mini Apps Partner Program: https://developer.apple.com/programs/mini-apps-partner/ (확인 2026-10-05)
- [A3] The Block, "Coinbase Wallet to remove DApp browser to comply with Apple's policy": https://www.theblock.co/linked/51693/coinbase-wallet-to-remove-dapp-browser-to-comply-with-apples-policy (2019-12-28)
- [A4] The Register, Apple–Tencent 15% mini app deal: https://www.theregister.com/2025/11/15/apple_tencent_app_deal/ (2025-11-15)
- [WK1] Apple, WKContentWorld: https://developer.apple.com/documentation/webkit/wkcontentworld ; OWASP MASTG https://mas.owasp.org/MASTG/knowledge/ios/MASVS-PLATFORM/MASTG-KNOW-0139/ (확인 2026-10-05)
- [AG1] MCP Registry(2025-09-08 프리뷰, 2025-10-24 API 동결): https://modelcontextprotocol.info/tools/registry/ (2차 정리, 확인 2026-10-05)
- [AG2] Coinbase, "Introducing x402 Bazaar": https://www.coinbase.com/developer-platform/discover/launches/x402-bazaar (2025-09)
- [K1] 헤럴드경제, "코인 레퍼럴 마케팅, 더 이상 회색지대가 아니다" (금융위 2025-12 판단 보도): https://www.heraldk.com/article/2026010416000001018 (2026-01-04)
- [K2] 파이낸셜뉴스, 가상자산 다단계·유사수신 수사 사례: https://www.fnnews.com/news/202104151652112463 (2021-04-15)
- [PH1] awesome-directories, "Product Hunt Launch Strategy 2025" (2차 출처, featured 감소 수치): https://scour.ing/p/https://awesome-directories.com/blog/product-hunt-launch-guide-2025-algorithm-changes (2025)

**출처 품질 주의:** [F4], [PH1], [AG1], [R1]은 2차 정리 자료다. [W4], [D1]은 본문 접근이 막혀 검색 요약에 기댔다. [K1]의 금융위 원문 공지는 직접 확인하지 못했으므로 변호사 검토 때 원문을 확보해야 한다. Blockaid 수치 [BL1]는 회사 자체 발표다.
