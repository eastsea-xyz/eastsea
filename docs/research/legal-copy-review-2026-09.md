# 사용자 대상 문구 법률 위험 검토 (2026-09-28)

> **법률 자문이 아닙니다.** AI가 [legal-review-2026.md](legal-review-2026.md)(그 자체도 변호사 의견이 아님)와 [15-node-rewards.md](../design/15-node-rewards.md) "초기 참여자가 알게 하기"를 기준으로 문구를 대조한 결과입니다. ⚖️ 표시는 메인넷 전에 **변호사 확인이 필요한 항목**입니다.

## 범위와 기준

- **대상:** `apps/wallet/Sources/*.swift`(Text·Label·help·alert·버튼), `apps/extension`(manifest, `ui/`, README), `agents/skills/aether-wallet/SKILL.md`, 저장소 `README.md`, 공개 README(`origin/main:README.md`), `docs/design/12-launch-plan.md` 중 공개로 이어지는 결정.
- **기준:**
  - 기대수익을 암시하는 투자·수익·이자·"earn" 표현(Howey 이익 기대, 자본시장법 제4조 제6항).
  - 가치 보장, "공짜 돈", "일찍 올수록 유리".
  - 탈중앙·보안 과장(legal-review §4 표).
  - 위험 고지 누락: 키 분실, 테스트넷 토큰 무가치·이월 없음, 무보증, 사용자 책임.
  - 메인넷 규칙 문장이 한 곳(`VotingRules.mainnetRewardsRule`)에서 나오는지.
  - 한국: 가상자산이용자보호법 제10조, 특금법 제2조·제7조, 자본시장법 증권성, 표시광고법 제3조. 미국: Howey, FTC Act §5.
  - 개인정보: DeviceCheck, 기기를 떠나는 데이터. App Store 3.1.5.
- NFT 가이드라인은 해당 기능이 없어 적용하지 않았습니다.

## 1. 위험 문구 표

줄 번호는 이 커밋 **이전**(`HEAD` 9a65911) 기준입니다. "적용"은 이번 커밋에서 고쳤다는 뜻입니다.

### 1-1. 지갑 앱 (`apps/wallet/Sources`)

| 파일:줄 | 지금 문구 | 위험 | 바꾼(제안) 문구 | 상태 |
|---|---|---|---|---|
| Onboarding.swift:20 `mainnetRewardsRule` | "On the future mainnet there is no token sale and no founder share. Block rewards go to the Macs that are online, every hour. …" | (1) "no founder share"는 창업자가 자기 Mac으로 받는 보상을 가림(legal-review §1 "창업자 배분이 없다는 것과 … 보유하지 않는다는 것은 다릅니다"). (2) 보상 절반은 증명 몫인데 "online Macs"만 적어 부정확. (3) 규칙이 확정된 것처럼 읽힘(발행 공식은 위원회 업그레이드로 변경 가능). (4) 테스트넷 이월 없음·현금화 없음이 빠짐 | "Planned for the future mainnet, which is not live: the rules may change before launch, and after it only by a committee-signed upgrade. No token sale, no premine and no founder allocation; the founder's Macs follow the same rules as everyone's. Half of each block's reward goes to registered Macs that stay online, shared every hour, and half to registered Macs that prove blocks. One operator gets at most 1/16 of each half, … Testnet AETH does not carry over. Nothing here promises a price, a return or a way to cash out." | 적용 |
| Onboarding.swift:6 `Terms.version = 1` | 동의 문구가 크게 바뀌었는데 버전 그대로 | 이미 동의한 사용자가 새 고지를 보지 못함 | `2`로 올림(다음 실행 때 한 번 다시 동의) | 적용 |
| Onboarding.swift:45 | "…has not been audited." | 정확하지만 "버그가 있을 수 있음"이 없음 | "…has not had an independent security audit, and may have bugs." | 적용 |
| Onboarding.swift:47 | "…at your own risk and responsibility, including following the laws where you live." | 전력·장비 비용, 세금 책임 누락(legal-review §4 "장비·전력 비용", §5-가 세금) | "…including power and hardware costs, taxes, and following the laws where you live." | 적용 |
| Onboarding.swift (동의 창) | 개인정보 고지 없음 | IP가 DHT·다른 노드에 노출, DeviceCheck 토큰이 Pipln 레지스트라와 Apple로 감, 주소·거래 공개. 개인정보보호법 제15·17·28조의8 고지 부족 ⚖️ | 새 항목: "Running Aether shows your IP address to other nodes and the public DHT. Joining as a voting node sends an Apple DeviceCheck token to the registration service, currently run by Pipln, which checks it with Apple. Addresses and transactions are public on chain." | 적용 |
| Onboarding.swift:79 (투표 노드 초대) | "Registration uses Apple DeviceCheck: one Mac, one voting node." | 등록 승인 주체(Pipln 레지스트라)를 숨김. legal-review §4 "등록 통제를 숨기지 않았는가" | "Registration sends an Apple DeviceCheck token to the registration service, currently run by Pipln, which checks with Apple that this is a real Mac: one Mac, one voting node. Touch ID signs it." | 적용 |
| Earnings.swift:152 | 상태 알약 "EARNING" | 수익 활동처럼 읽힘. 15번 "수익" 금지 | 증명 중엔 항상 "PROVING" | 적용 |
| Earnings.swift:242 | "Earned so far" | 동일 | "Received so far" | 적용 |
| Earnings.swift:414 | "Prove blocks on this Mac's GPU / to earn test AETH" | "Mac으로 돈 벌기" 유도 문구 | "…/ for test AETH rewards" | 적용 |
| Earnings.swift:688 | 메뉴 막대 "Earning · +x test AETH today" | 동일 | "Proving · +x test AETH today" | 적용 |
| Earnings.swift:223 | "The first valid proof of a block gets paid." | "paid"는 대가 지급·수입 인상 | "…gets a reward." | 적용 |
| Earnings.swift:225 | "Checking alone earns nothing yet on testnet." | "yet"이 곧 번다는 기대를 줌 | "Checking alone is not rewarded on testnet." | 적용 |
| Earnings.swift:425, 890 / AetherWalletApp.swift:109 | "…The first valid proof of a block is paid to this wallet." | 전력 비용 부담 미고지 | "Uses the GPU and power while on, at your cost. The first valid proof of a block gets a test AETH reward in this wallet." | 적용 |
| SimpleDashboard.swift:372 | "…can never be copied out. … There is no seed phrase to lose." | 분실 위험이 없는 것처럼 읽힘(키 분실 고지 누락) | "…If you lose every device and have no recovery set up, nobody can restore the funds." | 적용 |
| SimpleDashboard.swift:500 | DisclosureGroup "Mainnet reward rules" | 창업자 예비 키 공개 누락(12번 "창업자 Mac 안전망": "README와 앱에 공개합니다") | "Planned mainnet rules" + `VotingRules.founderReserveRule` 추가 | 적용 |
| SimpleDashboard.swift:540 | "…and no owner can hold a third." | 탈중앙 과장. 상한은 **주소** 단위라 실소유자를 식별하지 못함(legal-review §3-가) | "…no single operator address can hold a third of the seats." | 적용 |
| SimpleDashboard.swift:722 | "No server was trusted." | 과장. 앱에 실린 위원회 키와 레지스트라는 신뢰함 | "…using the committee key shipped with the app instead of a server's word." | 적용 |
| NetworkPaused.swift:5 | "…and nothing is lost." | 보장 표현 | "…; a pause by itself does not move funds." | 적용 |
| SimpleDashboard.swift:538–539 | "Your Mac proves it is online every hour, for free." | 뜻은 "수수료 없음"이지만 "free"가 무위험·공짜로 읽힐 소지(legal-review §4 한국 커뮤니티 "무료 채굴＝무위험") | "…every hour, with no fee." 제안 | 제안(낮음) |
| SimpleDashboard.swift:471–473 | "Updates install by themselves." | 사실이지만 업데이트 키를 Pipln이 쥔다는 점(legal-review §5-다, §7-6) 미고지 | 동의 창이나 설정에 "Updates are signed by Pipln and install automatically; you can check the release notes before …" 추가 검토 | 제안 |
| Earnings.swift:124, 247 (ConfettiBurst, FloatingReward, 금색 발광·카운트업) | 보상 도착 때 폭죽·금색 숫자 | 문구는 아니지만 메인넷에서 가치가 붙으면 "보상 = 돈"을 강조하는 연출로 볼 수 있음(FTC §5 순수 인상 기준) ⚖️ | 메인넷 빌드에서는 연출을 줄이고 수량만 표시하는 옵션 검토 | 제안 |
| EarningsText.unit = "test AETH" (Earnings.swift:51) | 메인넷에서도 이 단위를 쓰면 틀림 | 메인넷 전환 때 단위·"no value" 문구를 함께 바꿔야 함 | 메인넷 빌드 체크리스트에 추가 | 제안 |
| NodeController.swift | 사용자 문구는 오류 메시지뿐 | 위험 없음(다른 에이전트 작업 중이라 수정하지 않음) | — | — |

### 1-2. 저장소 README.md

| 줄 | 지금 문구 | 위험 | 바꾼 문구 | 상태 |
|---|---|---|---|---|
| 5 | "Any Mac can be a validator" | "누구나 즉시 검증자" 과장(legal-review §4 표). 실제로는 등록·24시간 연속·추첨 | "Apple silicon Macs can register as voting nodes" | 적용 |
| 10 | "has not been audited. All tokens (AETH) and rewards are test artifacts with zero monetary value." | 메인넷 AETH까지 가치 0이라고 단정하는 것처럼 읽힘(메인넷 계획과 충돌). 이월 없음 누락 | "has not had an independent security audit. On the testnet, AETH and all rewards are test tokens with zero monetary value, and they do not carry over to any mainnet. Nothing here is investment, legal or tax advice." | 적용 |
| 15 | "There is no seed phrase." | 키 분실 위험 누락 | 다음 줄 추가: "If you lose the device and have not set up a recovery key, nobody can restore the account." | 적용 |
| 19 | "It never takes a server's word for it." | 신뢰 가정(위원회 키) 숨김 | "It trusts the committee key it ships with, not a server's word." | 적용 |
| 69 | "No local file or process can get past them." | 절대적 보안 주장 | "Editing local files or processes does not lift them; only the owner can change them, with Touch ID." | 적용 |
| (없음) | 메인넷 규칙 섹션 없음 | 앱과 README 문장 불일치 | "Planned mainnet rules" 섹션: 앱 문장 그대로 인용 + 레지스트라(Pipln·Apple 비보증), 창업자 예비 키, 1/16이 주소 단위라는 한계 | 적용 |
| README.ko/zh-CN/ja/vi/es | 영어판 옛 문구의 번역 | 번역본이 같은 과장을 유지 | 영어판에 맞춰 다시 번역 | 제안 |

### 1-3. 공개 README (`origin/main:README.md`, 수정하지 않음)

| 줄 | 지금 문구 | 위험 | 제안 문구 |
|---|---|---|---|
| 3 | "Any Mac can be a validator" | 위와 같음 | "Apple silicon Macs can register as voting nodes, and your Mac verifies your wallet itself." |
| 8 | "All tokens (AETH) and rewards are test artifacts with zero monetary value." | 아래 "How mainnet will launch"와 한 페이지에서 충돌(메인넷 AETH는 가치 0이라고 약속할 수 없음) | "On the testnet, AETH and all rewards are test tokens with zero monetary value and do not carry over. Nothing here is investment, legal or tax advice." |
| 14 | "signed and notarized by Apple" | 공증을 Apple의 보증으로 오인(legal-review §4 "Apple이 보증하는", §5-나 "공증은 App Review가 아님") | "signed with Pipln's Developer ID and notarized by Apple (a malware check, not an endorsement)" |
| 20 | "There is no seed phrase." | 키 분실 고지 없음 | 다음 줄: "If you lose the device and have no recovery key or words, nobody can restore the account." |
| 23 | "It never takes a server's word for it." | 신뢰 가정 숨김 | "It trusts the committee key it ships with, not a server's word." |
| 35–44 "How mainnet will launch" | "Every AETH … under the same rules for everyone", "Rewards shrink slowly and never stop", "The chain counts this from the public registry; nobody signs for it." | (1) 창업자 예비 키 예외 미공개. (2) "never stop"은 영구 발행 약속처럼 읽힘(위원회 업그레이드로 변경 가능). (3) "nobody signs for it"은 N 계산에는 맞지만 등록에 Pipln 서명이 필요하다는 점을 가림. (4) 메인넷이 아직 없고 규칙이 바뀔 수 있다는 말이 없음 | 섹션 본문을 앱 문장(`VotingRules.mainnetRewardsRule`)으로 교체하고, 저장소 README의 "Registration / Founder reserve keys / What 1/16 does not do" 세 줄을 그대로 추가. "never stop" → "stops shrinking at 0.1 AETH a block under the current design" |
| 43 | "One Mac, one registration. … more shares need more real Macs." | 사실. 다만 등록 주체 미기재 | "Registering goes through a registration service, currently run by Pipln, that checks an Apple DeviceCheck token with Apple." |
| (없음) | 개인정보 고지 없음 | IP·DeviceCheck 토큰 처리 | 짧은 "Privacy" 항목: IP가 다른 노드·DHT에 보임, 투표 노드 등록 때 DeviceCheck 토큰이 Pipln 레지스트라→Apple, 체인 데이터 공개 |

### 1-4. 브라우저 확장 (`apps/extension`, 수정하지 않음)

| 파일:줄 | 지금 문구 | 위험 | 제안 |
|---|---|---|---|
| manifest.json:6 | "Your key never leaves this browser" | 사용자가 "Show private key"로 내보낼 수 있어 절대 표현이 틀림. 스토어 설명으로 쓰일 문장 | "Aether testnet wallet in your browser. Your key is stored encrypted in this browser; every transaction needs your approval. Test AETH has no value." |
| ui/popup.js:74 | "…This is the testnet: test AETH has no value." | 좋음. 무보증·미감사 고지가 없음 | "…Experimental software, provided as is and not independently audited." 추가 |
| ui/popup.js (홈 카드) | 잔액 표시에 검증 여부 없음 | README "Not yet": 확장은 노드 답을 검증 없이 표시. 앱과 달리 "Verified"가 아님을 알려야 함 | 잔액 아래 "Read from the node · not verified in the browser" (AssetsSheet.swift:37과 같은 방식) |
| Chrome 웹 스토어 설명(12번 "배포") | 아직 없음 | 스토어 설명에 가격·수익 표현 금지, 테스트넷 명시 | 위 manifest 문장 + DISCLAIMER 링크 |

### 1-5. `agents/skills/aether-wallet/SKILL.md` (수정하지 않음)

| 줄 | 지금 문구 | 위험 | 제안 |
|---|---|---|---|
| 11 | "Nothing the agent runs can get past them." | 절대적 보안 주장(컨트랙트 결함 가능성) | "The agent cannot change them; only the owner can, with Touch ID." |
| 9 | "It cannot be exported or copied, even by the agent." | Secure Enclave 특성상 사실. 유지 | — |
| 규칙 6 | DEX 견적은 추정치라고 말하게 함 | 좋음(가격 오인 방지). 유지 | — |

### 1-6. `docs/design/12-launch-plan.md` 공개로 이어지는 결정

| 줄 | 내용 | 위험 | 제안 |
|---|---|---|---|
| 92, 99 | "창업자는 어떤 몫도 받지 않습니다", "창업자 몫 없음" | 공개 문구로 옮길 때 "창업자는 AETH를 갖지 않는다"로 오인 가능. 창업자 Mac도 같은 규칙으로 받음 | 공개 문구는 "no founder allocation; the founder's Macs follow the same rules" 로 통일(이번 앱·README 적용분) |
| 105 | 창업자 예비 키 "README와 앱에 공개합니다" | 지금까지 미공개 | 이번에 앱(Network 화면 "Planned mainnet rules")과 저장소 README에 추가. 공개 README에도 추가 필요 |
| 106 | "일찍 열어도 누구도 초기 발행을 독식하지 못합니다" | 과장: 지갑 여러 개 + Mac 여러 대면 여러 몫(15번 A표 "과장하지 않는다") | "한 운영자 주소는 1/16을 넘지 못합니다"로 공개 문구 제한 |
| 108 | 유료 외부 감사 대신 AI 교차 감사 | 공개 문구에서 "audited"로 쓰면 legal-review §4 위반 | 공개 문구는 "not independently audited" 유지(이번 적용분과 일치) ⚖️ |
| 111 | "변호사 자문 대신 codex 자문" | 가장 큰 공백. legal-review §7-4도 "정식 법률 확인"을 메인넷 전 조건으로 둠 | 메인넷 제네시스 전 한국·미국 변호사 의견서 ⚖️ |
| 118 | "발행까지 운영자 n/16 … 한 명이 늘 때마다 모두가 발행에 가까워지는" | (1) 15번으로 대체된 옛 규칙(지금은 제네시스부터 발행). (2) 참여 모집을 발행 기대와 묶는 연출 | "운영자 N명 · 한 사람 최대 1/16 · 이번 에포크 분배율 x%"(15번 표와 같게) |
| 119 | "일찍 온 사람의 이점은 연속 참여 기록뿐" | 15번 "먼저 온 사람이 더 받는다"와 충돌. 어느 쪽도 공개 문구로 쓰면 안 됨 | 공개 문구에서는 초기 이점을 말하지 않고 규칙만 적기 |
| 15-node-rewards.md "초기 참여자의 이점" | "따로 규칙을 두지 않아도 먼저 온 사람이 더 받는다" | 내부 설계 메모지만 소스 공개 때 그대로 공개됨. 15번 자신이 금지한 "지금 들어오면 유리" 표현 | "운영자가 적을 때 한 사람 몫은 1/16, 많아지면 1/N" 같은 사실 서술로 교체 |

### 1-7. 저장소 DISCLAIMER.md (범위 밖, 참고)

앱 동의 창이 링크하는 문서라 함께 봤습니다.

| 위치 | 문구 | 위험 | 제안 |
|---|---|---|---|
| §2-2 | Tokens "possess no monetary value … cannot be redeemed" | 메인넷 계획과 충돌. 메인넷 전 반드시 개정 | "Testnet tokens …"로 범위 한정, 메인넷 조항 별도 |
| §2-3 | "does not constitute … VASP activity under any jurisdiction" | 법적 결론을 단정. 법원·규제기관을 구속하지 않고 오히려 과장 표시가 됨 ⚖️ | "is not intended as …" + 이용자 관할 확인 책임 |
| §1 | "decentralized" | 레지스트라 단독 키 존재 | "peer-to-peer, with a registration service currently run by Pipln" |
| 국문 번역본 | 시행일 2026-09-16 vs 영문 2026-09-28 | 버전 불일치 | 날짜·내용 동기화 |

## 2. 추가해야 할 고지 (누락)

| 고지 | 지금 | 조치 |
|---|---|---|
| 키 분실 = 자산 복구 불가 | 동의 창에만 있었음 | 보안 화면·README에 추가(적용). 공개 README 추가 필요 |
| 테스트넷 토큰 무가치·이월 없음 | 앱 동의 창·faucet은 있음, README는 "zero value"만 | 적용. 공개 README 8줄 교체 필요 |
| 무보증·미감사·버그 가능 | 있음(동의 창) | "independent security audit" 표현으로 통일(적용) |
| 전력·장비 비용, 세금은 사용자 부담 | 없음 | 동의 창·증명 도움말에 추가(적용). 보상 기록 내보내기(ProverMenu "Export Reward Records…")가 있음을 공개 README에 적기 제안 |
| 규칙은 계획이며 바뀔 수 있음 | 없음 | 메인넷 규칙 문장 첫머리에 추가(적용) |
| 레지스트라(Pipln)의 등록 통제 | 없음 | 초대 창·동의 창·README에 추가(적용). 등록 기준 공개 문서(`14-registration.md`) 링크를 공개 README에 제안 |
| 창업자 예비 키 | 없음 | 앱·README에 추가(적용). 공개 README 필요 |
| 개인정보: IP, DeviceCheck 토큰, 공개 체인 데이터 | 없음 | 동의 창에 추가(적용). 개인정보처리방침(처리 주체 Pipln, 보관기간, Apple로 국외 이전) 별도 문서 ⚖️ |
| 자동 업데이트 권한 | "Updates install by themselves"만 | 업데이트 서명 주체(Pipln)와 끄는 방법 고지 제안 |
| Apple은 후원·보증 주체가 아님 | 없음 | 저장소 README에 추가(적용). 공개 README "notarized by Apple" 옆에 필요 |
| 확장 잔액 미검증 | README "Not yet"에만 | 팝업에 표시 제안 |

## 3. 메인넷 규칙 문장의 일관성

- **한 곳:** `VotingRules.mainnetRewardsRule`(Onboarding.swift). 동의 창, 투표 노드 초대 창, Network 화면 "Planned mainnet rules"가 모두 이 상수를 씁니다.
- **README.md:** 같은 문장을 글자 그대로 인용하고 상수 이름을 적었습니다. 문장을 바꾸면 둘 다 바꿔야 합니다.
- **남은 불일치:**
  - 공개 README "How mainnet will launch"는 옛 문장을 씁니다.
  - 12번 118행은 옛 "발행까지 n/16" 규칙입니다.
- **권장:** 공개 README를 갱신할 때 이 문장으로 바꾸고, 15번 "초기 참여자가 알게 하기"의 보상 카드 문구("이 Mac은 보상을 받고 있습니다 …")도 같은 어휘(reward, received; earn 금지)로 구현합니다.

## 4. 한국 법 관점

- **자본시장법 제4조 제6항(투자계약증권):** 문구에서 "earn", "paid", 가격·수익 암시를 빼면 "타인 노력에 의한 이익 기대" 형성 위험이 줄어듭니다. 창업자 노력(개발·등록 승인)에 기대게 하는 문구는 이번 검토에서 찾지 못했습니다. ⚖️ 발행 구조 자체의 증권성은 문구로 해결되지 않습니다.
- **특금법 제2조·제7조(VASP):**
  - 앱과 확장은 비수탁 지갑입니다. 교환·중개 기능이 없습니다.
  - WalletModel은 "Swap tokens" 같은 **호출 내용 풀이**만 합니다.
  - 지갑 안에 DEX 매매 화면을 넣으면 다시 검토해야 합니다(12번 E3 원칙 유지). ⚖️
- **가상자산이용자보호법 제10조:**
  - 시세·거래량 관련 문구는 없습니다.
  - Pipln(창업자)이 자기 Mac 보상으로 AETH를 보유하게 되므로 제10조 제5항(자기 발행 가상자산 거래 제한)을 메인넷 전에 확인해야 합니다. ⚖️
- **표시광고법 제3조:**
  - "Any Mac can be a validator", "No server was trusted", "nothing is lost" 같은 과장·단정 표현을 고쳤습니다.
- **개인정보보호법:**
  - DeviceCheck 토큰, 운영자 주소, 노드 키, IP가 결합됩니다.
  - 레지스트라 로그의 보관기간과 Apple(미국) 전송 고지가 필요합니다(제28조의8 국외 이전). ⚖️

## 5. 미국 관점 (Howey, FTC)

- **Howey "이익 기대":** "EARNING", "Earned so far", "to earn" 제거로 줄였습니다. 폭죽·금색 카운트업 연출은 메인넷에서 재검토 대상입니다.
- **Howey "타인의 노력":** 등록이 Pipln 서비스에 의존한다는 점을 이제 숨기지 않습니다(숨기는 것보다 공개가 유리하다는 것이 legal-review의 권고). 의존성 자체는 남습니다. ⚖️
- **FTC §5:** 절대 표현("never", "No … can get past", "nothing is lost")을 조건 있는 사실 서술로 바꿨습니다.

## 6. App Store 3.1.5 (iOS)

| 항목 | 지금 | 판단 |
|---|---|---|
| (a) 지갑은 조직 계정 | `com.pipln.*` → Pipln 조직 팀(45WU468FZE) | 조직 계정으로 제출해야 함. 개인 팀으로 제출하면 거절 사유 |
| (b) 기기 내 채굴 금지 | 증명·노드·보상 화면이 모두 `#if os(macOS)`. iOS에서는 증명하지 않음(12번 116행) | 유지. iOS에서 증명·비콘을 켜지 않기 |
| (b) 보상 규칙 노출 | iOS 동의 창에도 `mainnetRewardsRule`(Mac 보상 규칙)이 보임 | 채굴 앱으로 오인될 소지. iOS 동의 창에서는 이 항목을 빼거나 "Rewards apply to the Mac app only" 한 줄로 줄이는 것을 제안(레이아웃 영향이 있어 이번에는 적용하지 않음) |
| (c) 거래소 기능은 인가 필요 | 지갑에 매매 기능 없음. 웹 페이지 요청(`aether://call`)으로 DEX 컨트랙트 호출에 서명은 가능 | 제3자 dApp 서명은 일반 지갑 기능이지만, 지갑이 DEX를 추천·연결하면 심사에서 문제될 수 있음 ⚖️ |
| (d) ICO·증권 거래 금지 | 없음 | — |
| (e) 앱 설치·홍보·게시 보상 금지 | faucet은 조건 없는 테스트 토큰. 추천·에어드롭 없음(12번 119행) | 유지. 추천 보상을 넣지 않기 |
| 개인정보 매니페스트 | `PrivacyInfo.xcprivacy` 없음(UserDefaults 등 required-reason API 사용) | TestFlight/App Store 제출 전 추가 필요 |
| 스토어 설명 | 아직 없음 | "testnet", "test AETH has no value", 수익·가격 표현 금지 |

## 7. 변호사 확인이 필요한 항목 (⚖️ 모음)

1. 메인넷 발행 구조(운영자 보상, 1/16 상한, 창업자 Mac 참여)의 한국 투자계약증권·미국 Howey 해당성. 문구 수정으로 해결되지 않습니다.
2. Pipln 레지스트라 운영의 개인정보 처리(처리 주체, 보관기간, Apple 국외 이전)와 개인정보처리방침.
3. 창업자(Pipln)가 보상으로 AETH를 보유할 때 가상자산이용자보호법 제10조 제5항과 이해관계 공시.
4. DISCLAIMER.md의 "no VASP under any jurisdiction" 같은 법적 단정과, 메인넷용 약관 개정.
5. 12번 111행의 "변호사 대신 AI 자문" 결정: 메인넷 전 정식 의견서로 바꿀 것.
6. iOS 앱에서 보상 규칙 노출·dApp 서명에 대한 App Review 대응.

## 8. 이번 커밋에서 바꾼 파일

- `apps/wallet/Sources/Onboarding.swift`: 규칙 문장 교체, `founderReserveRule` 추가, 약관 버전 2, 동의 창 3곳 수정·개인정보 항목 추가, 초대 창 등록 문구.
- `apps/wallet/Sources/Earnings.swift`: earn/paid 어휘 제거, 비용 고지.
- `apps/wallet/Sources/AetherWalletApp.swift`: 증명 설정 도움말.
- `apps/wallet/Sources/SimpleDashboard.swift`: 키 분실, 규칙 제목·예비 키, 1/3 상한 표현, 검증 도움말.
- `apps/wallet/Sources/NetworkPaused.swift`: "nothing is lost" 제거.
- `README.md`: 첫 문장, 경고 상자, 키 분실, 신뢰 가정, 에이전트 한도 표현, "Planned mainnet rules" 섹션.
- `NodeController.swift`, `crates/`, 공개 README, 확장, SKILL.md, DISCLAIMER.md는 바꾸지 않았습니다(위 표의 "제안").
