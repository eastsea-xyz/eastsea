# 데모 영상 두 편과 Show HN 초안

소스 공개 때 씁니다(결정 기록: 홍보는 소스 공개와 함께). 문구는 `docs/research/legal-review-2026.md`의 게시 전 체크리스트를 통과하도록 썼습니다. 공개 직전에 수치(확정 시간, 증명 시간)를 그날 측정값으로 다시 확인합니다.

## 영상 1. "시드 문구 없이, 10초 만에 첫 송금" (60초)

| 시간 | 화면 | 자막 |
|---|---|---|
| 0–5 s | Finder에서 Aether.dmg를 열고 앱을 Applications로 끌기 | Aether: Mac 전용 테스트넷 지갑 |
| 5–12 s | 앱 첫 실행. "계정 만들기" 한 번 클릭, Touch ID 창 | 키는 이 Mac의 Secure Enclave에서 만들어지고 밖으로 나오지 않습니다 |
| 12–15 s | 주소와 잔액 0이 보이는 대시보드 | 적어 둘 12단어가 없습니다 |
| 15–22 s | "테스트 토큰 받기" → 잔액 표시 | (테스트넷 전용 토큰, 가치 없음) |
| 22–35 s | 받는 주소 붙여넣기, 금액 1, 보내기 → Touch ID → "확정" | 서명은 Touch ID 한 번. 블록 확정까지 약 1초 |
| 35–45 s | 받은 쪽 두 번째 Mac 화면에 입금 표시 | 잔액은 서버가 아니라 이 Mac이 검증자 서명으로 직접 확인합니다 |
| 45–55 s | 설정 ▸ 복구: "복구 단어 만들기" 24단어, "다른 기기 신뢰" | 원하면 복구 단어나 다른 기기로 되찾기. 도둑이 시도하면 48시간 안에 취소 |
| 55–60 s | 로고, GitHub 주소 | 실험적 테스트넷입니다. 소스: github.com/… |

촬영 메모: 실제 시간을 편집으로 줄이지 않습니다(확정 1초를 그대로 보여 주는 것이 핵심). 테스트넷·가치 없음 표시는 화면에 계속 보이게 둡니다.

## 영상 2. "AI가 결제하되, 한도는 체인이 지킨다" (60초)

| 시간 | 화면 | 자막 |
|---|---|---|
| 0–8 s | 터미널: `aether-agent init` → Touch ID → "1회 1 AETH, 하루 3 AETH" | 한도는 주인이 Touch ID로 정하고, 계정 컨트랙트가 체인에서 강제합니다 |
| 8–15 s | `aether-agent setup all --apply` → Claude Code에 MCP 서버 "aether" 등록 | Claude Code, Codex 등 MCP 클라이언트 어디서나 |
| 15–30 s | Claude Code에 "Alice와 Bob에게 각각 0.5 AETH 보내 줘" → `aether_pay_many` → final: true | 한 트랜잭션, 둘 다 성공하거나 둘 다 실패 |
| 30–45 s | "그럼 Carol에게 5 AETH 보내 줘" → 정책 거절 → Claude가 한도와 변경 방법을 설명 | 에이전트가 무엇을 실행해도 한도를 넘을 수 없습니다(체인이 거절) |
| 45–55 s | 사람이 `aether-agent policy set --per-day 10` → Touch ID | 한도를 바꾸는 건 사람뿐입니다 |
| 55–60 s | 로고, 링크 | 에이전트 키도 Secure Enclave에. 꺼낼 수 없습니다 |

촬영 메모: 거절 장면에서 Claude가 우회(나눠 보내기)를 시도하지 않는 것까지 보여 줍니다(SKILL.md 규칙 3).

## Show HN

**제목:** Show HN: Aether – a Mac-only blockchain where your Mac verifies and proves the chain

**첫 문단:**

> Hi HN, I'm the founder of Aether (built with a small team at Pipln). It's an experimental layer-1 that only runs on Apple Silicon Macs, and a non-custodial wallet app. Your key is created in the Mac's Secure Enclave, so there is no seed phrase; you sign with Touch ID and blocks finalize in about a second. The wallet does not trust a server: it checks balances against the validators' threshold signature. Validators rotate every day or so and hand the key over without stopping the chain, so neither I nor anyone else can halt it. Blocks are proven with Jolt on the Mac's GPU (Metal): a 50-transaction block proves in about a minute on an M1 Max and verifies in 0.3 s. AI agents can pay through an MCP server, with per-payment and daily limits enforced by the account contract.
>
> It's a testnet: the token has no value, nothing is for sale, and there is no premine. Honest limits: joining as a validator candidate currently needs an Apple DeviceCheck check run by our registrar service (bounded by on-chain rules, replaceable by the validator committee); security review so far is AI cross-review, fuzzing and simulation, not an independent audit. Apple does not endorse this project. Code: <link>. Happy to answer anything about the consensus handoff, the stateless-witness proofs, or Secure Enclave accounts.

**체크리스트 적용 결과**

- [x] 작동하는 기능만 현재형으로 씀(증명 시장·발행은 언급하지 않음. 켜진 뒤에 추가)
- [x] 가격·수익·상장 표현 없음, "no value, nothing for sale, no premine"
- [x] 레지스트라 통제와 업그레이드 가능성을 숨기지 않음
- [x] 성능 수치는 측정 조건(M1 Max, 50 tx) 명시
- [x] 창업자 관계·소속(Pipln) 공개
- [x] 감사는 "AI 교차 검토·퍼징·시뮬레이션, 독립 감사 아님"으로 정확히 표시
- [x] "Apple does not endorse" 명시
