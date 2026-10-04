# site/ — EastSea 소개 페이지 (eastsea.xyz)

공개 원페이지 소개 사이트. **빌드 스텝 없음** — 순수 HTML/CSS/바닐라 JS 세 파일로
구성되고 어디에나 그대로 올리면 된다(이후 eastsea.xyz 호스팅 예정).

- `index.html` — 전체 콘텐츠(한국어·영어 동시 포함, CSS로 표시 전환)
- `styles.css` — 딥씨 팔레트(네이비·틸·골드), `prefers-color-scheme` 라이트/다크
- `README.md` — 이 문서

제약(지켜야 할 것): 외부 트래커 없음, 서드파티 CDN의 웹폰트·스크립트 없음(시스템
폰트 스택), 쿠키 없음, 이미지 없음(인라인 SVG뿐). 유일한 상태 저장은 언어 선택
`localStorage["eastsea-lang"]`(try/catch 감싸짐). 총 용량 약 52KB.

## 미리보기

```bash
python3 -m http.server -d site    # http://localhost:8000
```

## 동작 방식

- **언어**: `<head>` 안의 인라인 스크립트가 localStorage → `navigator.language`
  순으로 `ko`/`en`을 정해 `<html lang>`을 페인트 전에 설정한다.
  CSS는 `html:lang(ko) .en { display:none }`(반대도 동일)로 한쪽만 보여 준다.
  헤더의 토글 버튼이 lang을 뒤집고 localStorage에 저장한다. 네트워크 요청 0회.
- **다크 모드**: CSS 변수 + `prefers-color-scheme`. 별도 페이지 없음.
- **FAQ**: `<details>/<summary>` — 자바스크립트 없는 아코디언.
- **모션**: `prefers-reduced-motion: no-preference`일 때만 동작(코인 부유, 스파클).

## 검증 (2026-10-04, Playwright + Chrome)

360px·1280px × 라이트·다크 × ko·en 조합에서 확인했다:

- 가로 스크롤 없음(`scrollWidth == viewport`), 표도 360px 안에 들어옴
- 콘솔 에러·경고 0
- 언어 초기 판정(locale 기반)·토글·localStorage 저장 정상
- 본문 텍스트 대비 WCAG AA 전부 통과(라이트 최저 5.3:1, 다크 최저 7.9:1)

## 사실 ↔ 출처 매핑 (리드 검토용)

페이지의 모든 주장은 아래 레포 파일에서 나온다. **그 밖의 사실은 없음.**

| 페이지 문구(요지) | 출처 |
|---|---|
| "지갑도 노드도 내 맥 안에" / 시드 문구 없음 / Secure Enclave + Touch ID | `README.md` |
| 잔액을 서버에 묻지 않고 맥에서 검증(임계 서명·장부 증명) | `README.md`, `agents/skills/aether-wallet/SKILL.md` |
| VPN·포트 없이 집 인터넷으로 참여(DHT·iroh 홀펀칭) | `README.md` |
| 코인 이름 더블룬 Doubloon · DBLN | `apps/wallet/Sources/Brand.swift`, `TRADEMARKS.md` |
| 보상 개요: 매시간(에포크 3,600블록) 분배, 발행 절반은 노드 몫·절반은 증명 몫 | `docs/design/15-node-rewards.md` §규칙 |
| min(1/N, 1/16) 상한, 남는 몫 미발행 | 〃 |
| N=1/4/16+ 분배 표(1/16, 4/16, 전부) | 〃 §"운영자 N명이 모였을 때" |
| "먼저 온 사람이 더 큰 몫"(적을 때 1/16, 16 넘으면 1/N) | 〃 §초기 참여자의 이점 |
| 열두 번 예고 없는 점호, 답한 슬롯만큼(꼼수 차단) | 〃 §비콘 슬롯 |
| 워밍업 14일, 정상 몫의 절반에서 시작 | 〃 §B |
| 청구 없이 다음 에포크 첫 블록이 자동 지급 | 〃 §지급 시점 |
| 운영자 = 지갑 주소 단위 / 등록은 DeviceCheck로 맥 1대 1회 | 〃 §"16분의 1인지 어떻게 아나" |
| 출시 뒤 규칙 변경은 최소 7일 온체인 예고 업그레이드로만 | 〃 §규칙(604,800블록 예고) |
| 테스트넷 코인 무가치·미이관, 전기요금·세금 본인 부담 | `DISCLAIMER.md`, `docs/design/15-node-rewards.md` |
| 오픈소스 MIT · Apache-2.0 | `README.md`, `LICENSE-MIT`, `LICENSE-APACHE` |
| 프리마인·창업자 몫·토큰 세일 없음, 코인은 블록 보상로만 발행 | `README.md`, `DISCLAIMER.md` §2.1 |
| 메인넷 수도꼭지 없음(테스트 코인 수도꼭지는 테스트넷 전용) | `DISCLAIMER.md` §2.1, `agents/skills/aether-wallet/SKILL.md`(테스트 토큰 "testnet only") |
| 업데이트는 독립 빌더 2+(긴급 3) 서명, 발행 기록은 체인에 공개 | `docs/design/19-release-approval.md` |
| 보안 검토 여러 라운드(교차 모델)·"통과" 아님·유료 외부 감사 전 무 | `README.md`("not yet independently audited"), `docs/design/12-launch-plan.md` |
| 지갑 무분석·무개인정보수집 | `DISCLAIMER.md` |
| 필요 조건: 애플 실리콘 맥 1대 | `README.md` |
| 확장: Chrome·Edge·Brave·Arc, 키는 브라우저 생성·비밀번호 암호화 | `README.md` |
| 운영 주체 Pipln, 등록 서비스 + Apple DeviceCheck, Apple 비후원·비보증 | `README.md`, `AGENTS.md` |
| 맥 분실 시 복구 키 없으면 누구도 복구 불가 / 보조 Apple 기기 = 복구 키 | `README.md` |
| 히어로 상태 줄(테스트넷 운영 중·메인넷 전·세일 없음) | `README.md` |
| AI 에이전트 지갑: 한도·수신인·기한을 Touch ID로 설정, 체인이 강제, 속은 에이전트도 한도 내만 | `AGENTS.md`, `agents/skills/aether-wallet/SKILL.md` |
| 푸터 법적 문구("조언 아님"·as-is·DISCLAIMER.md 안내) | `DISCLAIMER.md` |
| 푸터 상표 문장(권리 주장·등록 준비 중) | `TRADEMARKS.md` |

의도적 생략: `docs/design/15-node-rewards.md`의 AETH 표기(레거시), 보관(sharding)·
증명 시장 상세, 예비 키·워밍업 세부 식, 발행 감쇠 곡선 수치. 소비자 페이지 범위 밖.

## 알려진 플레이스홀더

1. **다운로드 없음** — 두 카드 모두 "메인넷 출시와 함께 공개" 배지. 가짜 링크·버전 금지
   원칙에 따라 링크 자체를 두지 않았다. 출시 시 `#download` 섹션만 바꾸면 된다.
2. **소스 코드 링크 없음** — `docs/research/launch-posts-2026.md`에
   `github.com/aether-core/aether-mac` URL이 있으나 2026-10-04 현재 404(비공개).
   "출시와 함께 공개" 평문으로 대체. 공개되면 링크로 교체.
3. **`og:image` 없음** — 뒷받침할 이미지 URL이 없어 생략(지시: 없으면 태그를 넣지
   않는다). 호스팅 확정 후 1200×630 이미지를 만들어 추가 권장.
4. **"© 2026 Pipln"** — 파일 기준 연도. 호스팅 시점에 맞게 조정 필요.

## 유지 관리 규칙

- 상태 표시 줄(히어로)이 사실과 어긋나는 순간이 오면 **그 줄을 먼저** 고친다.
  (`mainnet has not launched` → 출시 후 문구 교체, `#download` 배지 제거)
- 새 사실을 넣을 때는 이 매핑 표에 출처를 함께 추가한다(없으면 넣지 않는다).
