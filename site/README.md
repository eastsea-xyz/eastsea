# site/: EastSea 소개 페이지 (eastsea.xyz)

공개 원페이지 소개 사이트다. 소개 페이지는 순수 HTML/CSS/바닐라 JS다. 같은 사이트의 `/explorer/`는 `scripts/build-extension.sh`와 `scripts/package-public-reader.sh`가 공유 검증기·공개 노드 읽기 리소스를 패키징한다 (자세히: `docs/ops/public-peer-reads.md`). Cloudflare Pages 프로젝트
`eastsea-site`가 이 폴더를 그대로 서빙한다. 2026-10-07에 "Dawn almanac" 방향으로 다시 디자인했다.
벤치마크와 결정 근거는 `docs/design/site-benchmark-2026-10-07.md`, 디자인 시스템은 `design/brand/SYSTEM.md`에 있다.

## 파일 목록

```
site/
├── index.html              — 전체 콘텐츠 (한국어·영어를 함께 담고 CSS로 하나만 표시)
├── privacy.html            — 개인정보 처리방침 (이중 언어, 같은 헤더·푸터)
├── tokens.css              — 생성물: design/brand/tokens.json → design/scripts/build-tokens.mjs (직접 수정 금지)
├── styles.css              — 레이아웃·컴포넌트. 색·글꼴·간격은 tokens.css 변수만 쓴다
├── assets/
│   ├── coin-{128,256,512,1024}.webp — 더블룬 코인 (design/brand/dbln-coin-1024.png에서 생성)
│   ├── app-icon-192.webp   — 내려받기 카드의 앱 아이콘 (design/brand/app-icon-1024.png)
│   ├── og-image.png        — OG 1200×630, design/og/og-image.html을 렌더한 것
│   ├── favicon.svg / favicon-32.png / apple-touch-icon.png (180)
└── fonts/  (모두 SIL OFL 1.1, 서브셋, 라이선스 전문 OFL-*.txt)
    ├── newsreader-latin.woff2   — 디스플레이 세리프 (wght 400–500, opsz 24–72) 59 KB
    ├── geist-latin.woff2        — 본문·UI 17 KB
    ├── geist-mono-latin.woff2   — 라벨·숫자·주소 12 KB
    └── hahmlet-ko-subset.woff2  — 한국어 제목용 세리프, 페이지 제목 글자만 43 KB
```

## 재생성

```bash
node design/scripts/build-tokens.mjs            # tokens.json → site/tokens.css (--check: 최신인지 확인)
python3 design/scripts/subset-ko-font.py <Hahmlet[wght].ttf>   # 제목 문구를 바꿨으면 반드시
node design/og/render-og.cjs                    # OG 이미지 (playwright 필요)
python3 -m http.server -d site                  # 미리보기 http://localhost:8000
```

**Hahmlet 서브셋 주의:** 한국어 제목(h1–h3, summary, td, blockquote)에 새 글자를 넣으면 서브셋을 다시 만들어야 한다.
다시 만들지 않으면 그 글자만 시스템 세리프로 떨어진다.

## 동작 방식

- **언어:** `<head>`의 인라인 스크립트가 localStorage를 먼저 보고, 없으면 `navigator.language`로 `ko`/`en`을 정한다.
  - 결과는 페인트 전에 `<html lang>`에 들어간다.
  - CSS `html:lang(ko) .en { display:none }`이 한쪽 언어만 보여 준다. 네트워크 요청은 없다.
- **다크 모드:** `prefers-color-scheme`를 따른다. tokens.css에는 `[data-theme]` 강제 지정도 들어 있다.
  - 낮(light)은 종이와 남색, 밤(dark)은 "밤바다"이며 주 버튼이 금색이 된다.
- **히어로 장면:** 해, 수평선, 판화 물결은 SVG와 CSS로 그린다. 지갑 창은 HTML/CSS 그림이다.
  - 승인된 지갑 리디자인 목업(`docs/design/wallet-redesign/mockups/01-dashboard-en.png`, `02-dashboard-ko.png`)을 그대로 옮겼다:
    사이드바, 코인이 든 남색 잔액 판, 같은 행 스타일, 같은 토큰 아이콘(더블룬 flat 마크, WAETH 공식 아트, 점선 링 + "?" 배지의 생성 글리프).
  - 창 안 문구는 모두 한국어·영어 두 벌이 있다. 체인 용어(블록 번호 등)는 쓰지 않는다 (지갑 스펙 P5).
  - 금액과 토큰은 예시라고 캡션에 밝혀 둔다.
  - 창 안은 앱과 같은 Apple 시스템 글꼴(SF Pro, Apple SD Gothic Neo)을 쓴다.
- **모션:** `prefers-reduced-motion: no-preference`일 때만 켜진다.
  - 해가 떠오르고, 햇살이 나타나고, 물결이 천천히 흐르고, 창이 떠오르고, 상단 점이 깜박인다.
- **외부 요청:** 소개 페이지는 없다 (CDN·분석·쿠키 0). `/explorer/`를 열면 내 노드를 먼저 시도한 뒤 공개 iroh 노드와 교체 가능한 WebSocket/pkarr 경로로 검증된 체인을 읽는다. 기본 HTTP 게이트웨이는 없다.
- **Lighthouse (2026-10-07, 로컬):**
  - 모바일: 성능 98, 접근성 100, 권장사항 100, SEO 100
  - 데스크톱: 성능 100
  - privacy.html: 성능 95, 접근성 100

## 2026-10-07 콘텐츠 개정 (사실 재점검)

- **내려받기:** 테스트넷 앱 0.7.0이 공개됐으므로 "다운로드 없음·앱 비공개"를 바꿨다.
  - 링크: https://github.com/eastsea-xyz/eastsea/releases/latest
  - 표기: Apple 실리콘, macOS 14+, Apple 공증, 테스트넷(체인 7780)
- **소스 코드:** 공개 저장소에는 릴리스와 DISCLAIMER만 있으므로 "앱 소스 미공개"를 유지한다.
  - MIT·Apache-2.0 공개는 계획으로 표시한다.
- **FAQ 테스트넷:** "팀 밖 참여 불가"를 "누구나 0.7.0으로 연결해 테스트 DBLN(faucet)으로 써 볼 수 있음"으로 바꿨다.
- **AI 에이전트:** "개발 중·미검증"을 "AI 에이전트 결제(aether-agent), 테스트넷 앱에 포함"으로 바꿨다.
  - 앱에는 Install Command-Line Tools 메뉴가 있다.
  - 기본 한도: 1회 1 DBLN, 24시간 10 DBLN, 7일.
  - "속은 에이전트도 한도 안에서는 쓸 수 있다"는 경고는 남겼다.
- **노드:** "켜 두면 매시간 점호에 응답"(보상은 계획)을 "스위치를 켜면 체인을 따라가며 모든 블록을 재실행"(README 사실)으로 바꿨다. 점호와 보상은 보상 섹션에서 "계획"으로 다룬다.
- **업데이트:** 오늘 사실은 "서명·Apple 공증 빌드로 자동 업데이트"다. 독립 빌더 2+ 서명은 메인넷 계획으로 명시했다.
- **보상 규칙 변경 절차:** README 문구에 맞춰 "7일 예고 업그레이드"를 "위원회 서명 업그레이드로만"으로 바꿨다.
- **툴박스 섹션 신설:** github.com/eastsea-xyz/eastsea-toolbox를 소개한다.
  - 예제와 테스트, ETH·Solana 호환성 증명과 벤치마크, 네이티브 설계를 담고 있다.
  - AS IS로 공개하며, Pipln은 배포·운영·호스팅·홍보를 하지 않고 수익·가격을 주장하지 않는다.
- **탐색기 링크 추가:** https://explorer.eastsea.xyz/
- **링크 정리:** 푸터 DISCLAIMER는 https://github.com/eastsea-xyz/eastsea/blob/main/DISCLAIMER.md, GitHub는 https://github.com/eastsea-xyz로 연결한다.
- **privacy.html:** privacy@eastsea.xyz가 동작하므로 TODO-confirm 자리표시자 안내를 삭제했다.
- **이름 정리:** 보이는 문구에서 Aether와 AETH를 없앴다. 남은 것은 CLI 이름 `aether-agent`뿐이며 사이트 본문에는 쓰지 않는다.
- **상표 문장:** Apple, Mac, Touch ID 상표 고지를 추가했다.

## 2026-10-07 리드 리뷰 반영 (2차)

- **언어 섞임 제거:** 히어로 앱 그림과 "작동 방식" 네 개의 작은 화면에 있던 영어 문구를 모두 한국어·영어 두 벌로 바꿨다.
  - 대상: 계정, 잔액 확인, 보내기·받기·자산, 최근 활동, 노드 스위치, 보내기 확인, 에이전트 한도 등.
  - 다운로드 카드 제목은 "Mac용 EastSea"로 바꿨다.
- **지갑 리디자인과 일치:** 히어로 창을 승인된 대시보드 목업(사이드바, 잔액 판, 행, 토큰 아이콘)과 같게 다시 그렸다.
- **체인 용어 제거 (P5):**
  - "block #184,233" → "이 Mac에서 확인함 · 방금 / Verified on this Mac · just now"
  - 증명 카드: "committee signature / state proof / block #" → "네트워크가 합의한 서명 / 장부에서 가져온 증명 / 이 Mac에서 직접 계산"
  - 02장 본문: "검증자 위원회의 서명·상태 증명" → "네트워크가 합의했다는 서명과 장부의 증명"
  - 03장 본문: "체인을 따라가며 모든 블록을 다시 실행" → "네트워크의 모든 거래를 직접 다시 계산해 확인"

## 사실 ↔ 출처 매핑 (리드 검토용)

페이지의 모든 주장은 아래 레포 파일이나 공개 릴리스에서 나온다. **그 밖의 사실은 없다.**
각 주장은 **오늘 사실**(지금 참인 것)과 **계획 설계**(메인넷을 위해 정해 둔 규칙과 의도) 가운데 하나로 구분한다.

| 페이지 문구(요지) | 상태 | 출처 |
|---|---|---|
| 상단 바 + 상태 장부: 공개 테스트넷(체인 7780) 운영 중 | 오늘 사실 | `README.md`, `DISCLAIMER.md` §1 |
| Mac 앱 0.7.0 공개(테스트넷), Apple 실리콘·macOS 14+, 공증, 자동 업데이트 | 오늘 사실 | GitHub 릴리스 `app-v0.7.0` 본문, eastsea-xyz/eastsea README |
| 앱 소스 코드 미공개(공개 저장소 = 릴리스 + DISCLAIMER) | 오늘 사실 | `gh api repos/eastsea-xyz/eastsea/contents` (README, DISCLAIMER.md만 있음) |
| 메인넷 미출시 | 오늘 사실 | `README.md`, `DISCLAIMER.md` §1 |
| 보안 결함 수정 중, 독립 외부 감사 없음 | 오늘 사실 | `README.md`, `DISCLAIMER.md` §1, `docs/design/12-launch-plan.md` |
| DBLN 판매 없음, 가격·수익 약속 없음 | 오늘 사실 | `README.md`, `DISCLAIMER.md` §2 |
| 테스트 DBLN 무가치·메인넷 미이관 | 오늘 사실 | `README.md`, `DISCLAIMER.md` §2 |
| 누구나 0.7.0으로 테스트넷 연결, 테스트 DBLN 받기 | 오늘 사실 | 릴리스 본문, `apps/wallet/Sources/ContentView.swift` ("Get 10 test …") |
| 시드 문구 없음, Secure Enclave + Touch ID, 복구 키(다른 Apple 기기) 미설정 시 복구 불가 | 오늘 사실 | `README.md` §What it is |
| 잔액마다 위원회 서명 + 상태 증명을 맥에서 검증 | 오늘 사실 | `README.md` (BLS threshold, EIP-7864) |
| 노드 스위치: 네트워크의 모든 거래를 다시 계산해 확인(= 체인 추종, 모든 블록 재실행) | 오늘 사실 | eastsea-xyz/eastsea README, `apps/wallet/Sources/NodeController.swift` |
| VPN·포트 없이 집 인터넷(DHT + QUIC 홀펀칭) | 오늘 사실 | `README.md` |
| AI 에이전트 결제: 수신인 승인 전 결제 꺼짐, 체인이 한도 집행, 기본 1/10 DBLN·7일 | 오늘 사실(테스트넷) | `README.md` §Wallet for AI agents, `AGENTS.md` |
| 지갑 무분석, 등록 외 개인정보 수집 없음 | 오늘 사실 | `DISCLAIMER.md` §4 item 5 |
| 탐색기 explorer.eastsea.xyz | 오늘 사실 | HTTP 200 (2026-10-07) |
| 툴박스: 예제+테스트, ETH·SOL 호환성 증명·벤치, 네이티브 설계, AS IS·비운영 | 오늘 사실 | eastsea-xyz/eastsea-toolbox README, 툴박스 공개 정책(2026-10-06) |
| 브라우저 확장(Chrome·Edge·Brave·Arc): 아직 배포 전 | 오늘 사실 | 0.7.0 릴리스 자산에 확장 없음, `README.md` §Browser extension |
| 운영 주체 Pipln, DeviceCheck 등록, Apple 비후원·비보증 | 오늘 사실 | `README.md`, `AGENTS.md` |
| 오픈소스 MIT·Apache-2.0 공개 | 계획 설계 | eastsea-xyz/eastsea README §Source code |
| 보상: 매시간, 절반 증명·절반 노드 몫, 1/16 상한, 남는 몫 미발행, 조기 혜택 없음 | 계획 설계 | `README.md` §Planned mainnet rules, `docs/design/15-node-rewards.md` |
| 열두 번 예고 없는 점호, 워밍업 2주 절반 | 계획 설계 | `docs/design/15-node-rewards.md` |
| 청구 없이 다음 시간 첫 블록이 지급 | 계획 설계 | `docs/design/15-node-rewards.md` §지급 시점 |
| 출시 뒤 규칙 변경은 위원회 서명 업그레이드로만 | 계획 설계 | `README.md` §Planned mainnet rules |
| 프리마인·창업자 몫·특별 배분 없음, 창업자 맥도 같은 규칙 | 계획 설계 | `README.md`, `DISCLAIMER.md` §2 |
| 메인넷 faucet 없음 | 계획 설계 | `DISCLAIMER.md` §2 |
| 독립 빌더 2+(긴급 3) 서명 배포 | 계획 설계 | `docs/design/19-release-approval.md` |
| 전기요금·세금 본인 부담 | 오늘 사실 | `DISCLAIMER.md` |
| 푸터 법적 문구(조언 아님, AS IS, 감사 없음, DISCLAIMER 링크) | 오늘 사실 | `DISCLAIMER.md` |
| 상표 문장 | 오늘 사실 | `TRADEMARKS.md` |

의도적으로 뺀 것: DEX·런치패드(사이트에서 Pipln이 DeFi를 운영한다는 인상을 주지 않도록), iPhone 앱(공개 배포 전), 발행 감쇠 수치.

## 알려진 남은 일

1. **브랜드 아트:** coin, favicon, app icon은 Codex 브랜치 `codex/brand-art`의 `design/brand/*`(작성 시점 미커밋)에서 가져왔다. 그 브랜치가 머지되면 원본 경로가 레포에 생긴다.
2. **코인 저작권:** AI 생성 이미지의 저작권 귀속은 변호사 확인이 필요하다 (`docs/ops/legal-open-questions.md`).
