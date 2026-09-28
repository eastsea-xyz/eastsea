# Aether 지갑 디자인 벤치마크와 정리안 (2026-09)

범위: `apps/wallet/Sources` (SimpleDashboard, Earnings, Onboarding, ContentView, AetherWalletApp, ProverMenu)와 iPhone 시뮬레이터 스크린샷(ios-home, ios-terms2, earnings-*, i16-network/security, se-home)을 기준으로 봤다.
결론 먼저: **"난잡함"의 원인은 기능이 많아서가 아니다. 같은 정보가 여러 곳에 반복되고, 수익 카드가 잔액보다 커졌고, 개발자·노드·법률 정보가 Simple 모드 첫 화면까지 새어 나왔기 때문이다.** 기능을 빼지 않아도 자리만 옮기면 절반은 해결된다.

---

## 1. 현재 인벤토리 (코드 기준 실측)

### 1.1 Home (Mac, 노드 ON 기준 · 위에서 아래 순)
| # | 블록 | 요소 | 비고 |
|---|---|---|---|
| 0 | 툴바 | Developer 토글 | 모든 페이지 공통 |
| 1 | `IncomingRecoveryAlert` | 제목, 설명, 버튼 2 | 조건부 |
| 2 | `NodeEarningsCard` → `EarningsHero` | LivePill, OrbitSpinner 오브, 대문자 라벨, **68pt heavy 숫자**, 시간당 델타 알약, StatTile 3개, 푸터 문장, "Prove blocks" CTA, 오로라 배경(30fps), 컨페티, 떠오르는 +보상 | **잔액보다 위, 잔액보다 큼** |
| 3 | hero | 계정 알약(아바타·이름·주소·복사 아이콘), **52pt 잔액**, Verified 배지 | |
| 4 | 액션 | Receive / Send / Get AETH (54pt 원형) | |
| 5 | `BalanceCard` | 변화량 라벨, 1H/1D/1W/All 세그먼트, 차트 170pt | |
| 6 | Tokens 카드 | 제목, 토큰 행 1개(Aether, "0 AETH"✓, "0") | 잔액 중복 |
| 7 | Recent activity 카드 | 제목, See all, 행 ≤3 | |
| S | 사이드바 하단 | 노드 토글+상태줄, EarningsBadge, 연결 점+Block # | |

- 합계: 카드형 표면 **5개**, 보이는 텍스트·컨트롤 **약 40개**, 동시에 움직이는 것 **4개**(오로라, 오브 스피너, LivePill 맥박, Verifying 스피너).
- 폰트 크기: 68 / 52 / 22 / 21 / 20 / headline / callout / caption / caption2 / 12 black tracking / 9.5 heavy tracking. **서로 다른 크기가 11종**이다(권장은 5~6종).
- 색: 브랜드 바이올렛, 바이올렛→핑크 그라데이션(아바타·토큰·오로라), 그린(Verified·델타·수신), 오렌지, 블루(송금), 퍼플(보안), 민트, 레드. **의미색이 7가지**이고 그린은 세 가지 의미로 쓰인다.

### 1.2 Home (iPhone) — ios-home.png, se-home.png
Developer 토글 → 계정 알약 → 0 AETH → Verified → 액션 3 → 차트 카드 → Tokens 카드 → (접힘 아래) Recent activity. 첫 화면 아래 절반을 **값이 0인 평평한 차트**가 차지하고, 초록 화살표로 "↗ +0 AETH in 1D"라고 표시한다. 잔액 0이 화면에 **세 번** 나온다(hero, 토큰 부제, 토큰 우측).

### 1.3 Network (Mac)
연결 카드(DHT·그룹 서명 설명 2줄) + 타일 5(블록, 검증자, 블록 시간, 수수료, 대기 tx) + 네트워크 활동 차트 카드 + **NodeCard**(토글, 설명, VotingNodeRow: 제목·상세·등록 상태·메인넷 규칙 캡션·Join) + **UpdateCard**(버전, 마지막 확인, 버튼). 표면 **9개**. 사용자의 노드·업데이트가 "네트워크 통계" 페이지에 섞여 있다.

### 1.4 Security (iPhone i16-security)
카드 3장. 본문은 약 45/70/40단어. "seed phrase가 없다"는 말이 두 카드에 연달아 나온다. 버튼은 모두 약한 텍스트 링크라 **뭘 해야 하는지가 문단 속에 묻혀 있다**.

### 1.5 같은 정보가 반복되는 곳
| 정보 | 나오는 곳 |
|---|---|
| 노드 ON/OFF 토글 | 사이드바, Network의 NodeCard, Settings, 메뉴바 → **4곳** |
| 최신 블록 번호 | 사이드바, Network 타일, Earnings 타일, 메뉴바 nodeLine, Developer 헤더 → **5곳** |
| Verified | Home 배지, 토큰 행 체크 씰, 메뉴바 → 3곳 |
| 메인넷 보상 규칙 긴 문장 | Terms 시트, 투표 노드 초대, VotingNodeRow 캡션 → 3곳 (문구가 서로 조금씩 다름; ios-terms2의 문장은 현재 `VotingRules.mainnetRewardsRule`과도 다르다) |
| 0 값 | 수익 카드 하나 안에서 "0 test AETH / Nothing in the last hour / +0 / 0 rewards / none yet" → **0을 다섯 번** |

---

## 2. 벤치마크 (2025-2026)

| 앱 | 첫 화면 위쪽 | 주요 액션 수 | 부가 기능을 두는 곳 | 보상·수익 표시 | 교훈 |
|---|---|---|---|---|---|
| **Phantom** | 총 잔액(USD) 1개 + 초록/빨강 %델타, 계정 이름·아바타 | 4 (Receive, Send, Swap, Buy), 모두 같은 크기 | 하단 탭, 토큰 상세 안 | 스테이킹은 **SOL 토큰 상세 → "Your Stake" → Last Reward**. 홈에는 없다 | 수익은 "자산 상세" 한 단계 아래 둔다 |
| **Cash App** (대조군) | "큰 잔액 숫자 하나, 경쟁하는 것 없음" | 2 | 탭·프로필 | 신뢰 배지(FDIC)는 아래에 조용히 | 배지는 작게, 아래에 |
| **Coinbase Wallet / Base app** | 2025년 Base app은 잔액 + 피드 + 배너 + 시세. 평가는 "과부하", "참여를 위한 설계이지 명료함이 아님" | 다수 | 소셜 피드, 미니앱 | 피드 속 보상·팁 | **2026-09에 Coinbase Wallet 이름으로 되돌리고 거래 중심으로 좁혔다.** 홈에 다 넣는 방식이 실패한 사례 |
| **Rainbow** | 잔액 + 자산 목록 | 3~4 | Points는 **별도 탭/화면** | 포인트 → RNBW 토큰. 리워드는 독립 공간 | 게임화 요소는 홈 밖에 둔다 |
| **Zerion** | 포트폴리오 + "Hot in Portfolio" | 3~4 | Rewards(XP, 레벨) = 별도 허브 | XP·퀘스트는 Rewards 허브 | 보상은 목적지로 만든다 |
| **Rabby** (모바일, 2025 말) | 멀티체인 잔액 + 기간별 변화 | 3~4 | 프로토콜 포지션은 목록 아래 | 서명 전 잔액 변화 미리보기 | 차트는 데이터가 있을 때만 |
| **Family** (Benji Taylor, 2026-02 종료 발표) | 잔액, 최소 액션 | 2~3 | **동적 트레이**(시트가 늘고 줄어듦) | — | "기본만 보이고, 나머지는 관련 있을 때 나타난다." 진행형 공개와 모션 연속성의 기준점. 종료 이유는 "목적이 분명한 경험이 범용 지갑보다 낫다"였다 |
| **Apple Wallet / Apple Cash** | 카드 이미지 + 잔액 한 줄 | 카드 안 2~3 (Send, Add money) | 카드 뒷면 "…" 메뉴 | Daily Cash는 **거래마다 한 줄**로 쌓이고, 잔액에 합쳐진다. 따로 된 영웅 카드는 없다 | 보상을 잔액의 한 줄 설명으로 흡수 |
| **Tailscale (macOS)** | 메뉴바: 연결 상태 + 핵심 토글. 창: 기기 검색 목록 | 메뉴바 1~2 | 2025 창 UI(1.96.2부터 기본): 검색, 오류, 디버그, 기능 발견은 창으로 옮김 | — | **메뉴바는 한눈 상태, 창은 관리.** 오류는 Dock 빨간 점 하나 |

공통 패턴
1. **위쪽에는 숫자 하나만.** 잔액이 가장 크고, 그 무엇도 잔액보다 크지 않다.
2. **주요 액션은 2~4개, 같은 크기.**
3. **수익·보상·XP는 홈의 한 줄 또는 별도 목적지.** 홈을 차지하는 영웅 카드로 쓰는 곳은 없다.
4. **0일 때는 보여주지 않는다.** 데이터가 생기면 나타난다(진행형 공개).
5. **신뢰 배지는 작고 조용하다.**
6. **메뉴바는 상태 + 토글 1개 + 열기.**

---

## 3. Aether 비판 (화면별 난잡함 목록)

### Home
- **H1. 수익 카드가 잔액을 이긴다.** 68pt heavy + 오로라 + 그림자 + 컨페티가 52pt 잔액 **위**에 있다. 사용자는 "내 돈"보다 "테스트 보상 0"을 먼저 본다. 벤치마크 앱 중 이렇게 하는 곳은 없다.
- **H2. 0의 과잉.** 수익 카드 안에서 0을 5번, 잔액 0을 3번 말한다. 평평한 차트 170pt와 "↗ +0 AETH" 초록 표시는 가짜 긍정 신호다.
- **H3. Tokens 카드가 쓸모없다.** 토큰이 AETH 하나뿐이라 잔액을 되풀이할 뿐이다.
- **H4. 움직임 4개가 동시에 돈다.** 오로라 30fps, 오브, 맥박 알약, Verifying 스피너. 주의를 뺏고 배터리를 쓰는 백그라운드 앱에 맞지 않는다.
- **H5. 카드가 5개인데 카드 컴포넌트는 3가지다**(`Card` 16pt 반경, `Tile` 14pt, `EarningsHero` 22/28pt + 흰 테두리). 한 화면에서 모양 규칙이 세 번 바뀐다.
- **H6. "Get AETH"가 Send와 같은 무게다.** 테스트넷에서는 괜찮지만, 잔액이 0일 때만 강조하고 잔액이 생기면 빠져야 한다.

### 전역
- **G1. Developer 토글이 iPhone 모든 화면 맨 위에 있다.** 소비자 첫 화면에 "Developer" 스위치를 두는 벤치마크 앱은 없다. 스크린샷(i16-*)에서는 라벨이 "Simple"로도 나와 켜짐/꺼짐 의미가 헷갈린다.
- **G2. 노드 토글 4곳, 블록 번호 5곳.** 한 곳(Node 화면)만 권위 있게 두고 나머지는 읽기 전용 요약으로 둔다.
- **G3. Network 페이지는 "체인 통계 + 내 노드 + 앱 업데이트"를 섞었다.** 일반 사용자는 수수료 0.000021, 대기 tx 0, 검증자 4를 알 필요가 없다. 이건 Developer 모드의 내용이다.

### Security
- **S1. 문단 3개가 행동을 가린다.** 사용자가 할 일은 두 가지(복구 단어 만들기, 복구 기기 신뢰)인데 150단어 뒤에 텍스트 링크로 숨어 있다.
- **S2. "seed phrase 없음"이 두 번 나온다.**

### 온보딩·법률
- **L1. Terms 시트가 5개 글머리, 약 150단어다.** 메인넷 보상 규칙(가장 긴 항목)은 첫 실행 사용자에게 가장 덜 중요하다. 보상 규칙은 Node 화면의 "규칙" 행으로 옮기고, 약관은 3줄 + 링크로 줄인다.
- **L2. 같은 규칙 문장이 3곳에 조금씩 다르게 있다.** 법률 문구는 한 곳에서만 관리해야 한다(`VotingRules.mainnetRewardsRule` 하나로).
- **L3. 투표 노드 초대가 5개 글머리 모달이다.** 동의가 필요한 3개(24시간 가동, DeviceCheck 1기기 1노드, 본인 책임)만 남긴다.

### 메뉴바 패널
- **M1. prover 상세 5줄**(Proving #, Proved # · tx · s, proofs · behind, Last reward, error)이 개발자용이다. 한 줄 요약 + "Open"이면 된다.

---

## 4. 제안: 정보 구조 (IA)

```
탭 / 사이드바 (4 → 3)
  Home      잔액 · 액션 3 · 수익 한 줄 · 최근 활동 3
  Activity  전체 기록 (수신/송금/보안/보상 필터 칩)
  Node      Mac: 이 Mac의 노드·증명·투표 좌석·보상 기록·규칙 / iPhone: "Mac에서 노드를 켜세요" 안내 + 연결 상태
  (설정 ⚙︎) 보안(복구 단어·복구 기기), 업데이트, 약관/규칙, Developer 모드, CLI 설치
```
- **Network 페이지는 없앤다.** 연결 상태 1줄은 Node로, 체인 통계 타일·네트워크 차트는 Developer 모드로.
- **Security는 설정 안의 "계정 보호" 섹션으로.** 복구 설정이 안 됐으면 Home에 **한 줄 경고 행**("복구 방법을 아직 정하지 않았습니다 ›")을 띄운다. 설정되면 사라진다.
- **Developer 토글은 설정 맨 아래로.** Mac은 View 메뉴의 ⌘⇧D도 가능.
- **iPhone 탭: Home · Activity · Node(또는 Settings)** 3개. 노드는 Mac 전용이므로 iPhone에서는 Settings가 3번째 탭이어도 된다.

### 카드 예산
| 화면 | 최대 카드형 표면 | 최대 주요 버튼 | 동시에 움직이는 것 |
|---|---|---|---|
| Home | **2** (수익 줄은 카드 아님, 활동 1) | 3 | 1 (새 보상이 들어올 때 한 번) |
| Activity | 1 목록 | 0 | 0 |
| Node | 3 (상태, 증명·보상, 투표 좌석) | 1 (상태에 따라 하나: 켜기/증명 시작/Join) | 1 (라이브 점) |
| Settings | 시스템 Form | — | 0 |
| 메뉴바 | 0 (구분선만) | 1 (Open) + 토글 1 | 1 (라이브 점) |

---

## 5. 수익 표시 규칙 (Earnings)

| 상태 | Home 표시 | Node 화면 |
|---|---|---|
| 노드 OFF | 없음 | "노드 켜기" 카드 1개 |
| 검증만 (보상 0) | 한 줄: `● 이 Mac이 블록을 확인하는 중` (회색 톤) | 상태 카드 + "GPU로 증명하기" 제안 1개 |
| 증명 중, 보상 0 | 한 줄: `● 증명 중 · 아직 보상 없음` | 증명 카드(진행 중 블록, 이번 세션 증명 수) |
| **보상 있음** | 잔액 바로 아래 한 줄: `✦ 오늘 +1.5 test AETH · 총 12.5 ›` (브랜드색) | **큰 수익 비주얼은 여기서만**: 누적 숫자, 오늘/지난 보상, 기록 |
| 새 보상 도착 | 한 줄이 한 번 반짝이고 잔액 숫자가 `numericText`로 올라감. 컨페티는 **그날 첫 보상** 또는 **첫 보상 평생 1회**에만 | 떠오르는 +보상 표시 |

원칙: **보상은 잔액의 설명문이다**(Apple Cash의 Daily Cash 방식). 영웅 카드(오로라·컨페티)는 Node 화면 전용이고, 보상 0 상태에서는 쓰지 않는다. `EarningsHero`를 버리자는 게 아니라 Home에서 Node로 옮기자는 것이다.

---

## 6. 디자인 토큰

**타입 스케일 (6단)**
| 토큰 | 크기/굵기 | 용도 |
|---|---|---|
| `display` | 48 semibold rounded, monospacedDigit | Home 잔액 (Mac 56) |
| `title` | 22 semibold | 페이지·시트 제목, Node 큰 숫자(Node만 40까지 허용) |
| `headline` | 17 semibold | 카드 제목 |
| `body` | 15 regular | 본문 (최대 2줄, 넘으면 "자세히") |
| `footnote` | 13 regular secondary | 부연, 시간 |
| `caption` | 11 medium | 배지, 라벨 (대문자 트래킹은 Node의 LivePill 한 곳만) |
→ 9.5pt heavy, 12pt black, 20/21pt는 없앤다.

**간격**: 4 / 8 / 12 / 16 / 24 / 32. 섹션 사이 24, 카드 안 16, 행 사이 12. 카드 반경 **16 하나로 통일**(타일 14, 영웅 28 제거).

**색 (의미를 하나씩만)**
| 토큰 | 값 | 의미 |
|---|---|---|
| `accent` | 기존 violet (0.49, 0.40, 0.95) | 버튼, 선택, 보상 줄 |
| `positive` | system green | **들어온 돈에만** (수신, +보상) |
| `warning` | system orange | 조치 필요 (복구 미설정, 배터리 일시정지) |
| `critical` | system red | 실패, 복구 공격 경고 |
| `neutral` | label / secondaryLabel / tertiary | 나머지 전부 |
→ Verified 배지는 초록 채움 대신 **secondary 텍스트 + 작은 체크**. 블루(송금)·퍼플(보안) 아이콘은 neutral로. 바이올렛→핑크 그라데이션은 아바타 한 곳에만.

**모션**: 상태 변화에만 (`.snappy` 0.25s). 계속 도는 것은 화면당 1개(라이브 점). 오로라는 Node 화면이 떠 있고 증명 중일 때만, `reduceMotion`이면 정지(이미 구현됨).

---

## 7. 와이어프레임

### 7.1 iPhone Home (보상 있음)
```
┌──────────────────────────────┐
│ ◉ Account 1  0x5397…E502 ⧉   │  ← footnote, 탭하면 복사
│                              │
│        12.5 AETH             │  ← display 48
│        ✓ Verified            │  ← caption, secondary
│  ✦ 오늘 +1.5 · 총 12.5  ›    │  ← accent 한 줄 (보상 0이면 숨김)
│                              │
│   (⤓)      (➤)      (💧)     │  ← 액션 3, 💧는 잔액 0일 때만 강조
│  Receive   Send   Get AETH   │
│                              │
│ ⚠ 복구 방법을 정하세요     › │  ← 미설정일 때만, warning 행
│                              │
│ 최근 활동             모두 › │
│ ↓ Reward  #184024   +0.5     │
│ ↓ Faucet  2분 전    +10      │
│ ↑ 0x12…ab 1시간 전  −2       │
├──────────────────────────────┤
│  Home    Activity    Node    │  ← 탭 3
└──────────────────────────────┘
```
빠진 것: Developer 토글, 차트 카드(Activity 상단으로 이동, 데이터 2점 이상일 때만), Tokens 카드(토큰이 2개 이상일 때 부활).

### 7.2 Mac Home (창 · 사이드바)
```
┌────────────┬──────────────────────────────────────────┐
│ Home       │   ◉ Account 1  0x5397…E502 ⧉             │
│ Activity   │                                          │
│ Node   ●   │            12.5 AETH                     │
│            │            ✓ Verified                    │
│            │   ✦ 오늘 +1.5 test AETH · 총 12.5   ›    │
│            │                                          │
│            │     (⤓ Receive) (➤ Send) (💧 Get AETH)   │
│            │                                          │
│            │  ┌ 최근 활동 ────────────────── 모두 › ┐ │
│            │  │ ↓ Reward #184024        +0.5       │ │
│            │  │ ↓ Faucet                +10        │ │
│            │  │ ↑ 0x12…ab               −2         │ │
│            │  └────────────────────────────────────┘ │
│ ─────────  │                                          │
│ ● 노드 켜짐 │                                          │  ← 사이드바 하단: 읽기 전용 1줄
│   증명 중   │                                          │     (클릭하면 Node로), 토글 없음
└────────────┴──────────────────────────────────────────┘
```

### 7.3 Mac Node
```
┌────────────┬──────────────────────────────────────────┐
│ Home       │ Node                          [ ON ◉ ]  │  ← 유일한 권위 있는 토글
│ Activity   │ ● 블록 #184210 확인 중 · 3시간 12분       │  ← 상태 한 줄 (연결 상태 포함)
│ Node   ●   │                                          │
│            │ ┌ 수익 (EarningsHero, 보상 있을 때만) ──┐ │
│            │ │ EARNING ●          12.5 test AETH     │ │
│            │ │ 오늘 +1.5 · 보상 24회 · 마지막 방금    │ │
│            │ └───────────────────────────────────────┘ │
│            │   (보상 0 → [⚡ GPU로 블록 증명하기] 카드) │
│            │                                          │
│            │ ┌ 투표 좌석 ───────────────────────────┐ │
│            │ │ 후보 · 5/24시간 연속 가동   [Join]    │ │
│            │ └───────────────────────────────────────┘ │
│            │                                          │
│            │ 설정 ›  보상 기록 내보내기 ›  규칙 ›      │  ← 텍스트 행 (규칙 문장은 여기 한 곳)
└────────────┴──────────────────────────────────────────┘
```
iPhone Node: "노드는 Mac에서 돌아갑니다. Mac에서 켜면 보상이 이 계정으로 들어옵니다." + 연결 상태 1줄 + (페어링된 Mac이 있으면) 그 Mac의 오늘 수익 1줄.

### 7.4 메뉴바 패널 (Tailscale식)
```
┌──────────────────────────────┐
│ 12.5 AETH            ✓       │
│ ✦ 오늘 +1.5 test AETH        │  ← 보상 있을 때만
│ ─────────────────────────── │
│ 이 Mac의 노드        [◉]     │
│ ● 증명 중 · 블록 #184210     │  ← 1줄 요약, 오류면 red 1줄
│ ─────────────────────────── │
│ [Aether 열기]     주소 복사 ⋯│
└──────────────────────────────┘
```
prover 상세 5줄 → 1줄. "Export Reward Records"는 Node 화면으로.

---

## 8. 첫 실행 흐름
1. **약관 시트 (3줄 + 링크)**: 실험용 테스트넷이며 보증 없음 / 테스트 AETH는 가치 없음 / 키는 이 기기에만 있음. [전체 약관 ›] [동의]. 메인넷 보상 규칙은 뺀다.
2. **Home** 빈 상태: 잔액 0 + "Get AETH" 강조 1개 + 빈 활동 안내 1줄. 차트·수익 줄 없음.
3. **첫 수신 후**: 복구 경고 행이 나타남("잔액이 생겼습니다. 복구 방법을 정하세요").
4. **Mac만, 노드가 따라잡은 뒤**: 투표 노드 초대 시트(글머리 3개). 규칙 전문은 "규칙 ›" 링크.
5. **첫 보상 순간**: 잔액 아래 수익 줄이 처음 나타나며 컨페티 1회. 이후 알림은 조용히.

---

## 9. 우선순위

### P0 — 빼거나 합치기 (반나절~하루, 구조 변경 없음)
1. Home에서 `NodeEarningsCard` 제거 → 잔액 아래 `EarningsMenuLine` 스타일 한 줄로 교체, 보상 0이면 숨김. (H1, H2)
2. iPhone 상단 Developer 토글 제거 → 설정/Network 페이지 맨 아래로. Mac 툴바 토글도 View 메뉴로. (G1)
3. Tokens 카드 숨김(토큰 1개일 때). (H3)
4. `BalanceCard`는 기록이 2점 이상이고 변화가 0이 아닐 때만 표시, "+0"에 초록 화살표 금지. (H2)
5. 사이드바 노드 토글을 읽기 전용 상태 1줄로, `EarningsSidebarBadge`와 연결 상태를 한 줄로 합침. (G2)
6. 메뉴바 prover 상세 5줄 → 1줄. (M1)
7. Terms 시트 5 → 3 글머리, 규칙 문장은 `VotingRules` 한 곳에서만. (L1, L2)
8. Security 카드 문단을 2줄로 자르고 "복구 단어 만들기"·"복구 기기 추가"를 채운 버튼으로. 중복 문장 삭제. (S1, S2)
9. 수익 카드 안 0 표시: 보상 0이면 StatTile 3개 대신 "아직 보상 없음" 한 줄. (H2)

### P1 — 구조 변경 (2~4일)
1. 탭 4 → 3: Network 폐지, **Node** 신설(Mac), 체인 통계는 Developer 모드로. (G3)
2. `EarningsHero`, NodeCard, VotingNodeRow, UpdateCard(→설정)를 Node 화면으로 이전.
3. Security를 설정 "계정 보호"로 옮기고, 미설정 시 Home 경고 행.
4. 디자인 토큰 파일(`Theme.swift`): 타입 6단, 간격, 반경 16, 의미색 5개. `Card`/`Tile`/영웅 반경 통일.
5. 모션 예산: 화면당 계속 도는 것 1개, 컨페티는 하루 첫 보상에만.
6. iPhone 3번째 탭: 페어링된 Mac의 노드 요약 (없으면 Mac 안내).

### 측정
- Home 보이는 요소 40 → **15 이하**, 카드 5 → **2**, 폰트 크기 11 → **6**.
- 5초 테스트: "지금 잔액은?", "오늘 번 것은?"에 답하는 시간.

---

## 출처
- Phantom 홈 위계·액션 4개, Coinbase "overloaded", Cash App 단일 숫자: [Crypto Wallet UX Teardown (Masterly)](https://www.themasterly.com/blog/crypto-wallet-ux-teardown)
- Phantom 톤("calm spacing, restraint with color and density"): [925 Studios – Phantom breakdown](https://www.925studios.co/blog/phantom-wallet-design-breakdown), 브랜드: [Phantom new brand identity](https://phantom.com/learn/blog/introducing-phantom-s-new-brand-identity)
- Phantom 스테이킹 보상 위치(Your Stake → Last Reward): [Phantom Help – Track native SOL staking rewards](https://help.phantom.com/hc/en-us/articles/4406644893971-Track-your-native-SOL-staking-rewards)
- Base app 2025 리브랜드: [CoinDesk](https://www.coindesk.com/tech/2025/07/17/coinbase-wallet-becomes-base-app-in-major-rebrand) / 2026-09 복귀: [The Crypto Times](https://www.cryptotimes.io/2026/09/11/base-app-returns-to-coinbase-wallet-a-year-after-rebranding/), [Crowdfund Insider](https://www.crowdfundinsider.com/2026/09/309263-coinbase-wallet-returns-as-base-app-social-experiment-ends/)
- Rainbow Points: [rainbow.me/points](https://rainbow.me/points), [Rainbow Rewards](https://rainbow.me/support/app/rainbow-rewards)
- Zerion Rewards(XP 허브): [Zerion blog](https://zerion.io/blog/rewards/)
- Rabby 모바일: [App Store](https://apps.apple.com/us/app/rabby-wallet-crypto-evm/id6474381673), [CryptoSlate review 2026](https://cryptoslate.com/crypto-wallets/rabby-wallet-review/)
- Family 원칙: [benji.org/family-values](https://benji.org/family-values), 종료: [The Block](https://www.theblock.co/post/388342/aave-labs-sunsets-avara-umbrella-brand)
- Apple Cash/Daily Cash: [Apple Cash](https://www.apple.com/apple-cash/), [Apple Support – Daily Cash](https://support.apple.com/en-us/119575)
- Tailscale 메뉴바 + 창 UI: [Escaping the notch](https://tailscale.com/blog/macos-notch-escape), [Windowed UI beta](https://tailscale.com/blog/windowed-macos-ui-beta)
- 메뉴바 HIG: [Apple HIG – The menu bar](https://developer.apple.com/design/human-interface-guidelines/the-menu-bar)
- 한계: Rabby·Zerion·Apple Wallet의 픽셀 단위 레이아웃은 공식 스크린샷을 직접 확인하지 못했다. 표의 해당 칸은 공식 설명과 스토어 소개를 바탕으로 쓴 것이다.
