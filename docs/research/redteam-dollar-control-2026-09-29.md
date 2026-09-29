# 레드팀: 달러를 옮기지 말고 통제권을 옮긴다 (Codex, 2026-09-29)

팀장 제안: USDC는 Base에 두고, Mac Secure Enclave 키와 불변 계정 컨트랙트로 한도·수취인·정지를 강제. 판정: 수정 후 추진.

task thread.
[codex] Thread ready (01a0eb8c-4c99-7943-9a3a-ca15c6183fff).
[codex] Turn started (01a0eb8c-4f9e-71c1-b9b6-c8b2064cc381).
[codex] Assistant message captured: 먼저 제안서와 지정된 저장소 문서를 읽고, Base·Circle·ERC-4337의 현재 사양은 공식 자료로 확인하겠습니다. 저장소와 임시 경로를 포함해 파일은 변경하지...
[codex] Running command: /bin/zsh -lc "sed -n '1,260p' contracts/src/AetherAccount.sol"
[codex] Running command: /bin/zsh -lc 'pwd && git status --short && ls -la .omx 2>/dev/null'
[codex] Running command: /bin/zsh -lc 'rg --files apps/agent | head -100'
[codex] Running command: /bin/zsh -lc "sed -n '1,260p' docs/research/redteam-strategy-2026-09-29.md"
[codex] Running command: /bin/zsh -lc "sed -n '1,260p' /private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73...
[codex] Command completed: /bin/zsh -lc "sed -n '1,260p' contracts/src/AetherAccount.sol" (exit 0)
[codex] Command completed: /bin/zsh -lc "sed -n '1,260p' /private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73... (exit 0)
[codex] Command completed: /bin/zsh -lc 'rg --files apps/agent | head -100' (exit 0)
[codex] Command completed: /bin/zsh -lc "sed -n '1,260p' docs/research/redteam-strategy-2026-09-29.md" (exit 0)
[codex] Command completed: /bin/zsh -lc 'pwd && git status --short && ls -la .omx 2>/dev/null' (exit 0)
[codex] Running command: /bin/zsh -lc "nl -ba docs/research/nextgen-bridge-2026.md | sed -n '1,280p'"
[codex] Running command: /bin/zsh -lc 'find .omx/state -maxdepth 2 -type f -print'
[codex] Running command: /bin/zsh -lc "nl -ba docs/research/stablecoins-2026.md | sed -n '1,320p'"
[codex] Running command: /bin/zsh -lc "nl -ba contracts/src/AetherAccount.sol | sed -n '260,520p'"
[codex] Running command: /bin/zsh -lc "nl -ba docs/design/21-crosschain.md | sed -n '1,300p'"
[codex] Running command: /bin/zsh -lc "nl -ba apps/agent/Sources/Keys.swift | sed -n '1,260p'; nl -ba apps/agent/Sourc...
[codex] Command completed: /bin/zsh -lc "nl -ba contracts/src/AetherAccount.sol | sed -n '260,520p'" (exit 0)
[codex] Command completed: /bin/zsh -lc "nl -ba docs/research/nextgen-bridge-2026.md | sed -n '1,280p'" (exit 0)
[codex] Command completed: /bin/zsh -lc "nl -ba apps/agent/Sources/Keys.swift | sed -n '1,260p'; nl -ba apps/agent/Sourc... (exit 0)
[codex] Command completed: /bin/zsh -lc "nl -ba docs/design/21-crosschain.md | sed -n '1,300p'" (exit 0)
[codex] Command completed: /bin/zsh -lc "nl -ba docs/research/stablecoins-2026.md | sed -n '1,320p'" (exit 0)
[codex] Command completed: /bin/zsh -lc 'find .omx/state -maxdepth 2 -type f -print' (exit 0)
[codex] Assistant message captured: 제안서는 P-256 키로 Base 계정을 직접 제어한다고 쓰지만, 현재 `AetherAccount.sol`의 세션 실행은 데이터 없는 기본 코인 전송만 허용합니다. U...
[codex] Running command: /bin/zsh -lc "nl -ba docs/research/stablecoins-2026.md | sed -n '1,240p'"
[codex] Running command: /bin/zsh -lc 'cat /Users/kjaylee/.agents/skills/agent-reach/SKILL.md'
[codex] Running command: /bin/zsh -lc "nl -ba docs/design/21-crosschain.md | sed -n '1,110p'"
[codex] Running command: /bin/zsh -lc 'cat .omx/state/session.json; cat .omx/state/skill-active-state.json'
[codex] Command completed: /bin/zsh -lc "nl -ba docs/research/stablecoins-2026.md | sed -n '1,240p'" (exit 0)
[codex] Command completed: /bin/zsh -lc 'cat /Users/kjaylee/.agents/skills/agent-reach/SKILL.md' (exit 0)
[codex] Command completed: /bin/zsh -lc "nl -ba docs/design/21-crosschain.md | sed -n '1,110p'" (exit 0)
[codex] Command completed: /bin/zsh -lc 'cat .omx/state/session.json; cat .omx/state/skill-active-state.json' (exit 0)
[codex] Assistant message captured: 공식 문서 확인에는 `agent-reach`의 웹 검색 경로를 적용하겠습니다. 이 환경에서 해당 경로가 막히면 제공된 웹 도구로 Base, Apple, Circle, ...
[codex] Running command: /bin/zsh -lc 'cat /Users/kjaylee/.agents/skills/agent-reach/references/search.md'
[codex] Running command: /bin/zsh -lc 'command -v agent-reach; command -v mcporter'
[codex] Running command: /bin/zsh -lc "nl -ba apps/agent/Sources/Tools.swift | sed -n '1,280p'"
[codex] Command completed: /bin/zsh -lc 'cat /Users/kjaylee/.agents/skills/agent-reach/references/search.md' (exit 0)
[codex] Command completed: /bin/zsh -lc 'command -v agent-reach; command -v mcporter' (exit 0)
[codex] Command completed: /bin/zsh -lc "nl -ba apps/agent/Sources/Tools.swift | sed -n '1,280p'" (exit 0)
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Assistant message captured: 공식 x402 사양에는 스마트 계정 경로가 있지만, 기본 결제 방식은 USDC의 EIP-3009 서명이고 별도 ERC-7710 위임 경로는 계정과 결제 대행자가 모두 ...
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Running command: /bin/zsh -lc 'mcporter call exa.web_search_exa query="Base Azul P256 precompile gas 6900 offi...
[codex] Command completed: /bin/zsh -lc 'mcporter call exa.web_search_exa query="Base Azul P256 precompile gas 6900 offi... (exit 0)
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Searching:
[codex] Assistant message captured: # 레드팀 검토 — 2026년 9월 29일 **판정은 ‘수정 후 추진’입니다.** USDC를 Base에 그대로 두면 Aether 브리지 금고와 래핑 달러의 위험을 피할...
[codex] Turn completion inferred after the main thread finished and subagent work drained.
# 레드팀 검토 — 2026년 9월 29일

**판정은 ‘수정 후 추진’입니다.** USDC를 Base에 그대로 두면 Aether 브리지 금고와 래핑 달러의 위험을 피할 수 있습니다. 다만 [팀장 제안서](/private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73-7af2-437d-8a06-ab75e6c24a76/scratchpad/dollar-plan.md:5)의 “같은 키로 직접 제어하고 같은 계약을 배포한다”는 구현 전제는 성립하지 않습니다. 아래의 심각도는 사실 자체의 위험보다, **현재 제안대로 출시할 때의 위험**을 평가한 것입니다.

## 사실 검증

**CRITICAL — Base의 P-256 지원과 계정 구조.** Base는 `0x100`에서 secp256r1 서명을 검증합니다. [Base 사양](https://docs.base.org/specifications/base-protocol/execution/precompiles)에 따르면 Fjord 때 검증당 3,450가스로 도입됐고, Azul 이후 비용은 **6,900가스**입니다. 이는 서명 검증 한 번의 비용이지 결제 거래 전체 비용이 아닙니다. 프리컴파일이 있다고 해서 P-256 키가 Base의 일반 거래나 EIP-7702 위임 인증에 직접 서명할 수 있는 것은 아닙니다. [EIP-7702](https://eips.ethereum.org/EIPS/eip-7702)는 해당 인증에 secp256k1 `ecrecover`를 사용하지만, 현 [AetherAccount.sol](/Volumes/workspace/aether-node/contracts/src/AetherAccount.sol:4)은 Aether의 P-256 계정이 EIP-7702로 코드를 위임한다는 전제와 `onlySelf` 호출에 의존합니다. **권고:** Base에는 P-256 공개키를 소유자로 등록하는 별도의 불변 *컨트랙트 계정*을 설계하십시오. ERC-4337 경로라면 배포·초기화와 `validateUserOp`도 필요합니다. 세션 정책의 개념은 옮길 수 있지만 현재 계약을 복사 배포할 수는 없습니다.

**HIGH — Secure Enclave 서명과 패스키의 혼동.** 현 앱은 [CryptoKit 키](/Volumes/workspace/aether-node/apps/agent/Sources/Keys.swift:40)로 임의 메시지에 **원시 P-256 서명**을 만들고 `r,s` 형태로 제출합니다([서명 경로](/Volumes/workspace/aether-node/apps/agent/Sources/Tools.swift:187)). 이는 WebAuthn assertion이 아닙니다. [WebAuthn 사양](https://www.w3.org/TR/webauthn-3/)의 assertion에는 `authenticatorData`와 `clientDataJSON` 검증이 따르며, [Coinbase Smart Wallet](https://github.com/coinbase/smart-wallet)의 패스키 소유자 경로도 `WebAuthnAuth`를 기대합니다. Base 프리컴파일은 어느 쪽이든 최종 P-256 서명을 검증할 수 있지만 WebAuthn의 출처·challenge·사용자 확인 규칙을 대신 검사하지 않습니다. **권고:** 기존 Mac 키를 유지한다면 원시 P-256용 계정 검증기를 명시하고 도메인·체인 ID·논스·서명 대상을 테스트하십시오. Coinbase 패스키 도구를 채택한다면 별도의 WebAuthn 자격증명과 검증 경로로 설계하십시오. 두 키가 같은 곡선을 쓴다는 이유로 동일한 키나 서명 형식으로 취급해서는 안 됩니다. 소유자 키의 현재 설정은 Touch ID **또는 로그인 암호**를 허용한다는 점도 소비자 설명에 반영해야 합니다([Keys.swift](/Volumes/workspace/aether-node/apps/agent/Sources/Keys.swift:27)).

**CRITICAL — x402 결제 호환성.** 원리상 스마트 계정으로 x402 결제는 가능합니다. [x402의 EVM `exact` 사양](https://github.com/x402-foundation/x402/blob/main/specs/schemes/exact/scheme_exact_evm.md)은 USDC의 EIP-3009 경로와 스마트 계정용 ERC-7710 위임 경로를 기술하며, [Circle 토큰 설계](https://github.com/circlefin/stablecoin-evm/blob/master/doc/tokendesign.md)는 ERC-1271 스마트 계정 서명을 지원합니다. 그러나 현재 [세션 실행](/Volumes/workspace/aether-node/contracts/src/AetherAccount.sol:358)은 호출 데이터가 있으면 거부하고 `Call.value`만 한도에 더합니다. 따라서 ERC-20 USDC 전송은 실행되지 않습니다. 더 위험한 수정은 호출 데이터만 허용하는 것입니다. 그러면 토큰의 실제 수취인·금액을 검사하지 못합니다. 또한 ERC-1271의 서명 검증만으로 EIP-3009 결제를 열 경우, 읽기 전용 검증 시점에 **누적 일일 지출을 기록할 수 없으므로** 현 세션의 하루 한도를 그대로 보장할 수 없습니다. 이는 인터페이스에서 도출한 설계 위험이며, 구체적 구현의 안전성 판정은 별도 검증이 필요합니다. **권고:** Base USDC 주소, 허용 함수, 수취인, 6자리 소수 단위 금액을 검사하고 지출을 상태에 기록하는 경로를 설계하십시오. 선택한 x402 결제 대행자가 그 경로를 실제로 검증·정산하는지 시험해야 합니다. 현 [MCP 도구](/Volumes/workspace/aether-node/apps/agent/Sources/Tools.swift:17)는 AETH 송금만 제공합니다.

**HIGH — ERC-4337 가스 부담.** Base에서 일반 결제의 가스는 ETH가 필요합니다. ERC-4337을 쓰면 계정이 ETH를 부담하거나, [EntryPoint에 ETH를 예치한 paymaster](https://eips.ethereum.org/EIPS/eip-4337)가 대납할 수 있습니다. 사용자가 USDC로 paymaster에 상환하는 제품은 가능하지만, 그 경우에도 실제 네트워크 가스가 USDC로 자동 결제되는 것은 아닙니다. x402의 기본 EIP-3009 흐름에서는 결제 대행자가 거래를 제출하고 가스를 부담합니다. [Coinbase CDP paymaster](https://docs.cdp.coinbase.com/paymaster/guides/wagmi-viem-integration)는 대납 수단이지만 설정·허용 목록·운영 비용에 의존합니다. Base의 [Cobalt 변경](https://docs.base.org/upgrades/cobalt/overview)은 B20 토큰 수수료를 다루지만, 이를 기존 ERC-20 USDC의 네이티브 가스 결제로 가정할 근거는 없습니다. **권고:** 계정 배포, 소유자 복구·정지, 일반 USDC 전송, x402 정산 각각의 가스 지불자를 정하고 paymaster가 없거나 거절할 때의 ETH 충전 경로를 마련하십시오. AETH 보상은 이 비용의 재원으로 계산하지 마십시오.

**HIGH — USDC 동결 노출.** 계정 계약에 관리자가 없어도 USDC 발행사의 권한은 남습니다. [Circle 계약 설계](https://github.com/circlefin/stablecoin-evm/blob/master/doc/tokendesign.md)는 주소 블랙리스트·일시 정지·업그레이드를 명시하고, [USDC 약관](https://www.circle.com/legal/usdc-terms)은 온체인 주소로의 입출금 차단 가능성을 설명합니다. 스마트 계정 주소가 차단되면 그 안의 USDC는 사실상 움직일 수 없으며, 복구 장치나 계정 불변성이 이를 해제하지 못합니다. **권고:** “키와 지출 규칙은 사용자가 통제한다”로 보호 범위를 정확히 설명하고, USDC의 발행사 동결 위험을 잔액·이용 약관에 분명히 표시하십시오. 전액을 한 계정에 둘지에 대한 노출 한도도 정해야 합니다.

**CRITICAL — Mac 분실·도난·초기화 후 복구.** [키 저장 코드](/Volumes/workspace/aether-node/apps/agent/Sources/Keys.swift:27)의 자료는 다른 Mac에서 사용할 수 없고, [README](/Volumes/workspace/aether-node/README.md:16)도 복구 키가 없으면 기기 손실 시 복구할 수 없다고 경고합니다. 계약에는 선택형 guardian과 지연 복구가 있지만([AetherAccount.sol](/Volumes/workspace/aether-node/contracts/src/AetherAccount.sol:149)), 현재 [초기화·소유자 명령](/Volumes/workspace/aether-node/apps/agent/Sources/Owner.swift:9)은 이를 필수 설정하거나 복구를 안내하지 않습니다. 현 EIP-7702 설계는 원본 키를 철회할 수도 없다고 스스로 명시합니다([AetherAccount.sol](/Volumes/workspace/aether-node/contracts/src/AetherAccount.sol:20)). **권고:** Base 컨트랙트 계정에는 철회 가능한 소유자 키와 독립 복구 수단을 넣고, 실자금 입금 전에 다른 기기에서의 복구 설정·리허설을 완료하게 하십시오. 기기 분실과 기기 탈취를 각각 시험해야 합니다.

## 공격면과 전략

**HIGH — Aether 체인의 소비자 가치.** 제안서대로면 USDC 구매의 서명, 정책 집행, 잔액, 정산이 모두 Base에서 끝납니다([제안서](/private/tmp/claude-501/-Volumes-workspace-aether-node/2274cf73-7af2-437d-8a06-ab75e6c24a76/scratchpad/dollar-plan.md:6)). 따라서 **소비자 결제에 Aether 합의가 필요 없다는 것은 설계로부터의 추론**입니다. 지갑 우선 제품이라는 목표라면 괜찮을 수 있지만, 이것만으로 Aether 체인의 수요를 증명하지는 못합니다. [기존 레드팀 검토](/Volumes/workspace/aether-node/docs/research/redteam-strategy-2026-09-29.md:105)도 AETH 보상에 가격·현금화 보장이 없음을 구분합니다. **권고:** Base 지갑의 성공 지표와 Aether 참여의 성공 지표를 분리하십시오. “체인이 결제를 가능하게 한다”는 설명은 쓰지 말고, 참여·보상·신원 기록 중 사용자가 실제로 선택할 기능을 따로 검증하십시오.

**HIGH — 법률·규제 노출.** 사용자만 키를 갖고 회사가 고객 자금을 받거나 이전하지 않는 지갑 소프트웨어라면 자기수탁 제공자라는 주장은 강해집니다. 다만 **그 사실만으로 송금업 해당 여부가 확정되지는 않습니다**. [FinCEN 지침](https://www.fincen.gov/system/files/2019-05/FinCEN%20Guidance%20CVC%20FINAL%20508.pdf)은 비수탁 지갑 사용과 단순 소프트웨어 개발을 구별하면서, 운영자가 실제로 가치를 수취·이전하는 사업을 하면 달리 평가합니다. 향후 Aether가 충전금, 환전, 공동 자금, 결제 대행 또는 고객을 대신한 이전을 운영하면 사실관계가 바뀝니다. 관할권별 판단도 필요합니다. [저장소 조사 문서](/Volumes/workspace/aether-node/docs/research/stablecoins-2026.md:40)의 “법적 책임 0%”는 검증된 결론으로 사용할 수 없습니다. **권고:** 출시 전 실제 자금 흐름·운영 주체·지원 국가를 그린 뒤 해당 관할의 전문가 검토를 받으십시오. 설계 문서와 소비자 문구에서 “규제 면제”를 삭제하십시오.

**CRITICAL — 프롬프트 인젝션.** Secure Enclave는 키 반출을 어렵게 하지만, 속은 에이전트가 *허용된* 결제에 서명하는 것은 막지 못합니다. 현재 기본 정책은 결제당 1 AETH, 하루 10 AETH이면서 수취인은 누구나, 만료는 없음입니다([Owner.swift](/Volumes/workspace/aether-node/apps/agent/Sources/Owner.swift:18)). 기존 [레드팀 검토](/Volumes/workspace/aether-node/docs/research/redteam-strategy-2026-09-29.md:93)도 한도 내 소진과 즉시 정지 부재를 지적합니다. 달러 한도로 옮겨도 공격 원리는 같습니다. **권고:** 기본값을 짧은 기간·명시한 수취인·작은 총예산으로 바꾸고, 구매 내용과 수취인을 사람에게 보여 주는 승인 기준을 두십시오. 즉시 세션 철회, 확정된 구매 내역, 환불 요청 경로를 출시 조건으로 삼으십시오.

**CRITICAL — Secure Enclave 단일 장애점.** 복구 미설정 상태에서는 Mac 한 대의 상실이 계정 접근 상실로 이어지고, 탈취된 세션 키는 철회 전까지 허용 범위에서 계속 지출할 수 있습니다. 이 위험은 앞의 복구 항목이 설명한 기술적 실패가 **사용자 잔액에 미치는 결과**입니다. **권고:** 서로 다른 기기·보관 장소의 복구 키, 지연과 통지, 복구 후 기존 소유자·세션 키 철회까지 한 흐름으로 시험하십시오. 복구가 준비되지 않은 계정에는 큰 USDC 잔액을 권하지 마십시오.

**HIGH — Base·Coinbase 의존성.** 이 안은 Aether 브리지와 초기 위원회에 대한 의존을 줄이는 대신 Base의 거래 포함·업그레이드·수수료와 Circle의 토큰 정책에 의존합니다. Coinbase의 번들러·paymaster까지 선택하면 서비스 정책과 가용성도 추가됩니다. Base가 위험을 줄여 온 것은 사실이며, [Base의 Stage 1 설명](https://blog.base.org/base-has-reached-stage-1-decentralization)은 fault proof와 업그레이드 승인 분산을 기술합니다. 따라서 이를 Coinbase 한 회사가 모든 자금을 임의로 관리하는 구조로 묘사해서도 안 됩니다. **권고:** 계정·서명·정책을 특정 호스팅 SDK와 분리하고, 대체 RPC·번들러·가스 대납 경로 및 계정 이주 절차를 마련하십시오. 이식성은 다른 체인에서 P-256·USDC·x402 상대가 모두 동작한다는 시험으로 입증해야 합니다.

**MEDIUM — 인텐트 브리지 등 대안과의 비교.** Base의 USDC로 Base 결제처에 지불하는 목표에는 달러를 옮기지 않는 안이 더 단순합니다. [ERC-7683](https://github.com/ethereum/ERCs/blob/master/ERCS/erc-7683.md)은 솔버 주문의 공통 형식이지 목적지 자산이나 정산 안전성을 보장하는 규격이 아닙니다. 저장소도 인텐트 선지급 후 Aether에서 보유하는 달러 사본의 위험이 남는다고 정정했습니다([nextgen-bridge-2026.md](/Volumes/workspace/aether-node/docs/research/nextgen-bridge-2026.md:1)). 반대로 **Aether 체인 안에서 달러를 사용해야 한다는 목표가 생기면** 이 안은 그 요구를 충족하지 못합니다. [교차체인 설계](/Volumes/workspace/aether-node/docs/design/21-crosschain.md:19)의 CCTP·브리지 경로는 그때 별도로 평가할 문제입니다. **권고:** 지금은 브리지 개발을 소비자용 USDC 결제의 선행 조건에서 빼고, Aether 내부 달러 수요와 발행사 지원이 실제로 확인될 때 재평가하십시오.

## 최종 판정

**방향은 유지하되, 현재 제안 그대로의 구현·실자금 출시는 진행하지 마십시오.** 출시 전 필요한 수정은 ① Base 전용 불변 P-256 컨트랙트 계정, ② 상태에 기록되는 USDC 세션 한도와 수취인 검사, ③ 선택한 x402 결제 경로의 실제 정산 검증, ④ 가스 지불자와 실패 시 대안, ⑤ 기기 상실·탈취 복구 및 즉시 정지, ⑥ Circle 동결과 관할별 법률 위험의 정확한 소비자 설명입니다. 그 뒤에야 “달러를 옮기지 않고 통제권을 제공한다”는 약속을 검증 가능한 제품 주장으로 만들 수 있습니다.

이 검토는 지정 문서·코드와 공식 사양을 읽어 수행했습니다. 파일은 수정하지 않았으며 Base 배포나 실결제 실행 시험은 하지 않았습니다.
