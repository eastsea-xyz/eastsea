# site/ — EastSea 소개 페이지 (eastsea.xyz)

공개 원페이지 소개 사이트. **빌드 스텝 없음** — 순수 HTML/CSS/바닐라 JS로 구성되고
어디에나 그대로 올리면 된다(이후 eastsea.xyz 호스팅 예정). v2(site-v2 브랜치)부터
실제 코인 렌더 이미지와 Split Horizon 디자인 방향이 적용됐고, 이후 Pretendard(1.5 MB)는
제거하고 시스템 한국어 스택 + EB Garamond italic(EN h1만)로 정리됐다.

## 파일 목록

```
site/
├── index.html              — 전체 콘텐츠 (한국어·영어 동시 포함, CSS display 전환)
├── privacy.html            — 개인정보 처리방침 (이중 언어)
├── styles.css              — Tide Tables 팔레트, prefers-color-scheme 라이트/다크
├── README.md               — 이 문서
├── assets/
│   ├── coin-1024.webp      — 더블룬 코인 렌더, 알파 컷아웃 (272KB) — 히어로 srcset
│   ├── coin-512.webp       — 코인 중간 해상도 (96KB)
│   ├── coin-256.webp       — 코인 저해상도 (32KB)
│   ├── coin-128.webp       — 코인 썸네일 (12KB)
│   ├── og-image.png        — OG 소셜 미리보기 (1200×630, 440KB)
│   ├── favicon-32.png      — 파비콘 (4KB)
│   └── apple-touch-icon.png— 애플 터치 아이콘 180px (56KB)
└── fonts/
    └── EBGaramond-Italic.woff2   — 영어 display h1 italic 400 (21KB, SIL OFL)
```

## 코인 에셋 출처 및 저작권 고지

`design/assets/doubloon-master-v1.png` (2048px 원본) 및 파생 webp 파일들은
이미지 생성 모델(생성일: 2026-10-04)로 제작되었다. 사용한 프롬프트는
`design/coin.md` §Image Generation Prompts Prompt A에 기록되어 있다.

**⚠️ AI 생성 미술의 저작권 소유자가 누구인지(Pipln, 운영자, 또는 없음)는 현재 법적으로
불명확하다.** `docs/ops/legal-open-questions.md`의 변호사 질의 목록에 포함되어
메인넷 출시 전 확인이 필요하다.

## 추가 에셋 (design/assets/)

```
design/assets/
├── doubloon-master-v1.png  — 원본 2048px 렌더 (5.7MB, RGB, alpha 없음)
├── app-icon-1024.png       — macOS 앱 아이콘 후보 (navy rounded-square + coin)
└── coin-1024.webp … (위와 동일)
```

## 미리보기

```bash
python3 -m http.server -d site    # http://localhost:8000
```

## 파일 크기 (HTML+CSS+JS)

| 파일 | 크기 |
|------|------|
| index.html | 36 KB |
| styles.css | 17 KB |
| privacy.html | 14 KB |
| **HTML+CSS 합계** | **~54 KB** (150 KB 예산 충분) |

폰트·이미지는 별도 파일로 위 예산에 포함되지 않는다.

## 동작 방식

- **언어**: `<head>` 안의 인라인 스크립트가 localStorage → `navigator.language`
  순으로 `ko`/`en`을 정해 `<html lang>`을 페인트 전에 설정한다.
  CSS는 `html:lang(ko) .en { display:none }`으로 한쪽만 보여 준다.
  헤더 토글 버튼이 lang을 뒤집고 localStorage에 저장한다. 네트워크 요청 0회.
- **다크 모드**: CSS 변수 + `prefers-color-scheme`. 별도 페이지 없음.
- **코인 이미지**: `<img srcset>` 3단계 (256/512/1024). filter: drop-shadow로 리프트.
- **타이포그래피**: 시스템 한국어 스택 + EB Garamond italic (EN h1만).
- **FAQ**: `<details>/<summary>` — JS 없는 아코디언.
- **모션**: `prefers-reduced-motion: no-preference`일 때만 — 코인 float(4s), glow pulse, light sweep.

## 검증 (2026-10-04, 사실 점검 리비전)

`index.html`·`privacy.html`을 360px·1280px × 라이트·다크 × ko·en 조합에서 확인:

- 가로 스크롤 없음, 외부 요청 0
- 컨솔 에러·경고 0
- EB Garamond italic이 영문 h1에 적용됨 (hero-en-garamond-check.png 참고)
- 본문 대비: 라이트 ink #16293a / bg #f5f0e8 → 11.4:1, 다크 ink #dce8f0 / bg #0b1622 → 13.1:1 (WCAG AA 초과)
- 버튼 btn-gold #d4a038 / btn-ink #1b2c3d → 6.2:1 (AA 통과)
- 사실 점검 리비전(같은 날, glm/site-truth): 상태 블록·계획 규칙 화법·개발 중 배지 반영 후
  360/1280 × 라이트·다크 재렌더, ko/en 문장쌍 정합 확인, HTML 파스·외부 요청 0 확인.

## 사실 ↔ 출처 매핑 (리드 검토용)

페이지의 모든 주장은 아래 레포 파일에서 나온다. **그 밖의 사실은 없음.**
각 주장은 **오늘 사실**(지금 참인 것) 또는 **계획 설계**(메인넷을 위해 정해 둔 규칙·의도)으로 구분한다.

| 페이지 문구(요지) | 상태 | 출처 |
|---|---|---|
| 히어로 상태 블록: 시험용 네트워크 운영 중 | 오늘 사실 | `README.md` (체인 7780 운영 중), `docs/launch/teaser-plan.md` |
| 앱과 소스 코드는 아직 비공개 · 다운로드 없음 | 오늘 사실 | `docs/launch/teaser-plan.md` "말할 수 있는 것", 다운로드 카드는 링크 없음 |
| 메인넷 미출시 | 오늘 사실 | `README.md` ("Mainnet has not launched yet") |
| 보안 결함 수정 중(내부 AI 교차 검토에서 Critical/High 발견) | 오늘 사실 | `docs/launch/teaser-plan.md` 결정 4, `docs/design/12-launch-plan.md` §보안 감사 |
| 공인 외부 보안 업체의 독립 감사 없음 | 오늘 사실 | `README.md`, `docs/design/12-launch-plan.md` §보안 감사, `docs/research/legal-opinion-memo-2026-10-04.md` §3.6 |
| DBLN 판매 없음 | 오늘 사실 | `README.md`, `DISCLAIMER.md` §2 |
| 테스트넷 코인 무가치·메인넷 미이관 | 오늘 사실 | `README.md`, `DISCLAIMER.md` §2 |
| FAQ 테스트넷: 팀 밖에서는 참여 불가(앱 비공개) | 오늘 사실 | `docs/launch/teaser-plan.md` "말하지 않는 것" |
| 시드 문구 없음 / Secure Enclave + Touch ID / 복구 키 미설정 시 복구 불가 | 오늘 사실 (비공개 빌드의 구현) | `README.md` |
| 받은 잔액 증명을 맥이 직접 검증("서버에 묻지 않는다" 아님) | 오늘 사실 (비공개 빌드의 구현) | `README.md`, `agents/skills/aether-wallet/SKILL.md` |
| VPN·포트 없이 집 인터넷으로 참여(DHT·iroh 홀펀칭) | 오늘 사실 (비공개 빌드의 구현) | `README.md` |
| 지갑 무분석·무개인정보수집 | 오늘 사실 | `DISCLAIMER.md` §4 |
| 필요 조건: 애플 실리콘 맥 1대 | 오늘 사실 | `README.md` |
| 확장: Chrome·Edge·Brave·Arc, 키는 브라우저 생성·비밀번호 암호화 | 오늘 사실 (비공개 빌드의 구현) | `README.md` |
| 운영 주체 Pipln, 등록 서비스 + Apple DeviceCheck, Apple 비후원·비보증 | 오늘 사실 | `README.md`, `AGENTS.md` |
| 코인 이름 더블룬 Doubloon · DBLN | 오늘 사실 | `apps/wallet/Sources/Brand.swift`, `TRADEMARKS.md` |
| 오픈소스 MIT·Apache-2.0 **공개 예정**(지금은 저장소 비공개) | 계획 설계 | `README.md`, `LICENSE-MIT`, `LICENSE-APACHE`, `docs/design/12-launch-plan.md` §빠른 메인넷(소스 동시 공개) |
| 보상 개요: 매시간 분배, 발행 절반은 증명 몫·절반은 노드 몫 | 계획 설계 | `docs/design/15-node-rewards.md` §규칙, `README.md` §Planned mainnet rules |
| min(1/N, 1/16) 상한, 남는 몫 미발행 | 계획 설계 | 〃 |
| N=1/4/16+ 분배 표(1/16, 4/16, 전부) | 계획 설계 | 〃 §"운영자 N명이 모였을 때" |
| "몫은 참여자 수에 따라 정해집니다" + "조기 참여 별도 혜택 없음" | 계획 설계 | 〃 §운영자 N명이 모였을 때(공식 그대로) |
| 열두 번 예고 없는 점호, 답한 슬롯만큼(꼼수 차단) | 계획 설계 | 〃 §비콘 슬롯 |
| 워밍업 14일, 정상 몫의 절반에서 시작 | 계획 설계 | 〃 §B |
| 청구 없이 다음 에포크 첫 블록이 자동 지급 | 계획 설계 | 〃 §지급 시점 |
| 운영자 = 지갑 주소 단위 / 등록은 DeviceCheck로 맥 1대 1회 | 계획 설계 | 〃 §"16분의 1인지 어떻게 아나" |
| 출시 뒤 규칙 변경은 최소 7일 온체인 예고 업그레이드로만 | 계획 설계 | 〃 §규칙(604,800블록 예고) |
| 프리마인·창업자 몫·특별 배분 없음, 코인은 블록 보상로만 발행 | 계획 설계 | `README.md` §Planned mainnet rules, `DISCLAIMER.md` §2 |
| 메인넷 수도꼭지 없음(테스트 코인 faucet은 테스트넷 전용) | 계획 설계 | `DISCLAIMER.md` §2, `agents/skills/aether-wallet/SKILL.md` (test tokens testnet only) |
| 업데이트는 독립 빌더 2+(긴급 3) 서명, 발행 기록은 체인에 공개 — "설계" | 계획 설계 | `docs/design/19-release-approval.md` |
| AI 에이전트 용돈 지갑 — "개발 중", 실제 Touch ID·네트워크 결제 미검증 명시 | 계획 설계 (개발 중) | `AGENTS.md`, `agents/skills/aether-wallet/SKILL.md`, `apps/agent` |
| 전기요금·세금 본인 부담 | 오늘 사실 | `DISCLAIMER.md`, `docs/design/15-node-rewards.md` |
| "켜 두면 매시간 점호에 응합니다"·가동 시간 기록(야간 수익 표현 없음) | 계획 설계 | `docs/design/15-node-rewards.md` §비콘 슬롯 |
| FAQ 토큰 세일: 창업자도 같은 규칙으로 맥을 가동해야 보상 | 계획 설계 | `README.md` §Planned mainnet rules |
| CTA "작동 방식 읽기"(앵커) — 주장 아님 | — | 페이지 내 #benefits 앵커 |
| 푸터 법적 문구("조언 아님"·as-is·DISCLAIMER.md 안내) | 오늘 사실 | `DISCLAIMER.md` |
| privacy.html 전체(수집 항목·목적·보유·Apple 이전·권리) | 오늘 사실(등록 서비스 운영 기준) | `docs/ops/privacy-policy.md`, `docs/design/14-registration.md`, `DISCLAIMER.md` §4 |
| 푸터 상표 문장(권리 주장·등록 준비 중) | 오늘 사실 | `TRADEMARKS.md` |

의도적 생략: 보관(sharding)·증명 시장 상세, 예비 키·워밍업 세부 식, 발행 감쇠 곡선
수치, 감사 라운드 횟수. 소비자 페이지 범위 밖.
"오늘 사실 (비공개 빌드의 구현)"은 지금 팀 빌드에서 구현돼 있으나 앱이 비공개라
외부 이용자가 당장 확인할 수 없음을 뜻한다.

## 알려진 플레이스홀더

1. **다운로드 없음** — 두 카드 모두 "메인넷 출시와 함께 공개" 배지.
   출시 시 `#download` 섹션만 교체.
2. **소스 코드 링크 없음** — 2026-10-04 현재 비공개. "출시와 함께 공개" 평문으로 대체.
   공개되면 링크로 교체.
3. **privacy.html 이메일** — `privacy@eastsea.xyz`는 TODO-confirm 자리표시자.
   실제 수신 주소로 교체 후 공개.
4. **코인 저작권** — AI 생성 이미지 저작권 귀속 → 변호사 확인 필요(legal-open-questions.md).

## 유지 관리 규칙

- 상태 표시 줄(히어로)이 사실과 어긋나는 순간이 오면 **그 줄을 먼저** 고친다.
- 새 사실을 넣을 때는 이 매핑 표에 출처를 함께 추가한다(없으면 넣지 않는다).
