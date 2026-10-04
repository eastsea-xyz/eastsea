# 데모 영상 두 편과 Show HN 초안

소스 공개 때 씁니다(결정 기록: 홍보는 소스 공개와 함께). 문구는 `docs/research/legal-review-2026.md`의 게시 전 체크리스트를 통과하도록 썼습니다. 공개 직전에 수치(확정 시간, 증명 시간)를 그날 측정값으로 다시 확인합니다.

## 영상 1. "시드 문구 없이, 10초 만에 첫 송금" (60초)

| 시간 | 화면 | 자막 |
|---|---|---|
| 0–5 s | Finder에서 EastSea.dmg를 열고 앱을 Applications로 끌기 | EastSea: Mac 전용 테스트넷 지갑 |
| 5–12 s | 앱 첫 실행. 누를 버튼 없이 계정이 바로 만들어짐 | 키는 첫 실행 때 이 Mac의 Secure Enclave에서 저절로 만들어지고 밖으로 나오지 않습니다 |
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
| 0–8 s | 터미널: `aether-agent init` → Touch ID → "1회 1 DBLN, 하루 10 DBLN" | 한도는 주인이 Touch ID로 정하고, 계정 컨트랙트가 체인에서 강제합니다 |
| 8–15 s | `aether-agent setup all --apply` → Claude Code에 MCP 서버 "aether" 등록 | Claude Code, Codex 등 MCP 클라이언트 어디서나 |
| 15–30 s | Claude Code에 "Alice와 Bob에게 각각 0.5 DBLN 보내 줘" → `aether_pay_many` → final: true | 한 트랜잭션, 둘 다 성공하거나 둘 다 실패 |
| 30–45 s | "그럼 Carol에게 5 DBLN 보내 줘" → 정책 거절 → Claude가 한도와 변경 방법을 설명 | 에이전트가 무엇을 실행해도 한도를 넘을 수 없습니다(체인이 거절) |
| 45–55 s | 사람이 `aether-agent policy set --per-tx 5 --per-day 10` → Touch ID | 한도를 바꾸는 건 사람뿐입니다 |
| 55–60 s | 로고, 링크 | 에이전트 키도 Secure Enclave에. 꺼낼 수 없습니다 |

촬영 전 준비: `aether-agent init`을 한 번 실행해 에이전트 키를 만들고, `aether-agent get-test-tokens`로 에이전트 계정을 채웁니다(faucet 10 DBLN). `init`은 에이전트 계정 잔액이 0.5 DBLN 이상일 때만 기본 한도(1회 1, 하루 10 DBLN)를 설정하고, 그보다 적으면 충전 안내만 출력합니다. 영상의 `init`은 충전한 뒤의 두 번째 실행입니다.

촬영 메모: 거절 장면에서 Claude가 우회(나눠 보내기)를 시도하지 않는 것까지 보여 줍니다(SKILL.md 규칙 3).

## Show HN

**제목:** Show HN: EastSea – a Mac-only blockchain where your Mac verifies and proves the chain

**첫 문단:**

> Hi HN, I'm the founder of EastSea (built with a small team at Pipln). It's an experimental layer-1 that only runs on Apple Silicon Macs, and a non-custodial wallet app. Your key is created in the Mac's Secure Enclave, so there is no seed phrase; you sign with Touch ID and blocks finalize in about a second. The wallet does not trust a server: it checks balances against the validators' threshold signature. Validators rotate every day or so and hand the key over without stopping the chain. It is designed so that no single operator, me included, can halt it: each draw gives one operator at most f of the 3f+1 voting seats, and finality needs 2f+1. Today all the testnet voting Macs are mine, so that only becomes true as outside operators join. From protocol 2, which is activating on the testnet, blocks are also proven with Jolt on the Mac's GPU (Metal): a 50-transaction block proves in about a minute on an M1 Max and verifies in 0.3 s. AI agents can pay through an MCP server, with per-payment and daily limits enforced by the account contract.
>
> It's a testnet: the token has no value and nothing is for sale. The testnet has a faucet; a mainnet genesis would have no premine. Honest limits: joining as a validator candidate needs an Apple DeviceCheck check run by our registrar service (from protocol 2, activating on the testnet, it is bounded by on-chain rules and the validator committee can replace or stop its key); operators are told apart by the wallet that registered each Mac, so one person with several Macs and wallets can get around the per-operator cap; security review so far is AI cross-review, randomized robustness tests and simulation, not an independent audit. Apple does not endorse this project. Code: <link>. Happy to answer anything about the consensus handoff, the stateless-witness proofs, or Secure Enclave accounts.

**체크리스트 적용 결과**

- [x] 작동하는 기능만 현재형으로 씀. 증명과 레지스트라 상한은 "from protocol 2, activating on the testnet"으로 씀. 발행·보상은 언급하지 않음(켜진 뒤에 추가). 활성화가 끝나면 이 문구를 현재형으로 바꿈
- [x] "아무도 멈출 수 없다"고 쓰지 않음. 투표 Mac이 모두 창업자 것인 동안은 "그렇게 설계했다"와 그 이유(운영자당 f석 상한, 2f+1 정족수)만 씀
- [x] 가격·수익·상장 표현 없음, "no value, nothing for sale". premine은 "testnet은 faucet, mainnet 제네시스는 사전 발행 0"으로 구분
- [x] 레지스트라 통제와 업그레이드 가능성을 숨기지 않음
- [x] 성능 수치는 측정 조건(M1 Max, 50 tx) 명시
- [x] 창업자 관계·소속(Pipln) 공개
- [x] 감사는 "AI 교차 검토·무작위 견고성 테스트·시뮬레이션, 독립 감사 아님"으로 정확히 표시(cargo-fuzz 타깃은 아직 없음)
- [x] "Apple does not endorse" 명시
