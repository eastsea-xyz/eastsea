# Aether 토큰경제 후속 조사: 2024–2026 최신 연구 반영 (2026-09-26)

선행 문서: `docs/research/tokenomics.md` (2026-09, 옵션 A–D, 권고 D = burn-only + points/work receipts, A 메커니즘).

**조사 방법.** WebSearch는 세션 한도(200/200)를 이미 다 써서 쓰지 못했습니다. 대신 arXiv API(`export.arxiv.org/api/query`)로 제목·초록을 직접 조회했고, Exa로 검색했으며, 1차 출처(EIP, SIMD, sec.gov, ethresear.ch, 공식 docs)는 직접 fetch했습니다. 1차 출처에서 확인하지 못한 항목은 **(unverified)** 로 표시했습니다. 법률 관련 기술은 법률 자문이 아닙니다.

코드 기준점: `crates/execution/src/block.rs`의 `GasVector { exec, state, prove }`와 `ProveGasMeter`(명령어당 1 unit), 그리고 `b.basefee = 0`, `beneficiary = leader_address(..)`(`crates/node/src/chain.rs:233`). 즉 **prove 차원 계량은 이미 있고, 가격만 매겨지지 않은 상태**입니다.

---

## 1. 요약: 9월 보고서 대비 달라진 점

1. **Base fee 소각의 이론적 근거가 더 탄탄해졌습니다. 다만 "완벽한 TFM은 없다"는 불가능성 결과도 함께 강화됐습니다.** Ganesh–Thomas–Weinberg(2024, 2025)는 off-chain influence proofness를 "burn identity"로 특성화했고, Chung–Roughgarden–Shi(2024)와 Gafni(2025)는 **2인 담합만으로도 담합 취약 메커니즘 전체를 공략할 수 있다**고 보였습니다. 소수 committee 체인은 담합이 쉬운 환경입니다. 그래서 **평소에는 사실상 posted-price로 동작하게 설계하는 것**(Ferreira–Gafni–Resnick 2024)이 현실적인 최선입니다.
2. **Proving 비용은 별도 차원으로 가격을 매겨야 합니다.** Chaliasos 외(2025)는 prover-killer 트랜잭션으로 rollup finality 지연이 **약 94배** 늘어남을 실측했습니다. EF zkEVM 팀도 2025-12~2026-01에 "block proving 기준 repricing" 작업을 공개했습니다. 9월 보고서에는 이 차원이 없었습니다. Aether는 이미 `GasVector.prove`를 계량하므로 여기에 base fee만 붙이면 됩니다.
3. **다차원 수수료는 Ethereum에서 "단일 `max_fee` + 차원별 base fee" 방향으로 정리되는 중입니다.** EIP-7999(2025-08, Draft)와 EIP-8011(2025-08, Draft)이 그 예입니다. 차원별 벡터 수수료인 EIP-7706은 Stagnant입니다. Reserve price 개념(EIP-7918, **Final**)도 확인했습니다.
4. **Prover 시장의 흐름은 "bond 없는 개방 제출 + 사후 지급"입니다.** Aztec은 2025-05에 대형 prover bond와 견적 방식을 버리고 **무허가 제출, 부분 epoch 증명, 보상 균등 분할**로 전환했습니다. Bahrani–Neuder(2025)는 담보가 작으면(B < V/ln V) **lottery가 staked allocation보다 낫다**고 보였고, Garimidi–Neuder–Roughgarden(2026)은 winner-take-all 조달이 집중을 부른다며 비독식 할당을 제안했습니다. 따라서 9월 옵션 A의 "Mina식: proposer가 증명을 사야 블록이 유효"는 **증명이 뒤따라오는 Aether에는 맞지 않습니다.** **프로토콜 escrow에 적립해 두고, 증명된 chunk에 대해 나중에 청구**하게 해야 합니다.
5. **Priority fee 일부 소각이 Ethereum에서도 공식 제안으로 올라왔습니다.** EIP-8375(2026-08, Draft)는 self-build를 포함한 모든 payload에서 priority fee의 일정 비율을 소각합니다. 9월 보고서의 "tip burn floor" 권고를 뒷받침합니다.
6. **Issuance:** Solana SIMD-0228은 참여율 74%로도 가결 기준 66.67%에 못 미쳐 **부결**됐습니다. 대안인 SIMD-0411(2025-11)은 SIMD-0550(2026-06, Review)으로 이름을 바꿔 재상정됐습니다. Elowsson(2026-06)은 저·무·음 issuance에서도 "offset/penalty"로 합의 인센티브의 상대 구조를 유지할 수 있다고 보였습니다. 결론은 **가치 0인 소규모 permissioned committee 체인에서 issuance는 보안을 사지 못합니다.** 불필요합니다.
7. **Alpenglow 경제(SIMD-0326/0357)가 확정됐습니다.** VAT는 epoch당 1.6 SOL을 **전액 소각**하고, 활성 집합은 상위 2,000으로 제한하며, **epoch 보상이 0인 노드는 제거**합니다. Committee 활동성은 "보상"이 아니라 "제거"로 강제한다는 패턴이 검증된 셈입니다.
8. **규제:** 미국 SEC는 2026-03-17 해석 릴리스(33-11412, 2026-03-23 발효)로 protocol mining과 staking이 증권이 아니며, **대가 없는 airdrop은 "investment of money" 요건을 충족하지 않는다**고 공식화했습니다. 이어 Regulation Crypto Assets(airdrop safe harbor 포함)를 **제안**했습니다. 한국 디지털자산기본법은 **2026-09 현재 미통과**입니다(공청회 추진, 11월 법안소위 예정). 결론적으로 "비양도 points, 전환 약속 없음" 원칙은 그대로 유지합니다.

---

## 2. 문헌 표 (최신순)

| # | 제목 | 저자 | 연도(월) | Venue / 상태 | 한 줄 결론 | URL |
|---|---|---|---|---|---|---|
| 1 | EIP-8375: ePBS Mandatory Burn of Execution Rewards | Ben Adams | 2026-08 | EIP Draft (requires 7732) | self-build를 포함한 모든 payload에서 priority fee의 고정 비율을 소각하고, 외부 builder bid에는 같은 비율의 burn target을 둠 | https://raw.githubusercontent.com/benaadams/EIPs/ember/EIPS/eip-8375.md , PR https://github.com/ethereum/EIPs/pull/12130 |
| 2 | ePBS, distilled | (ethresear.ch) | 2026-08 | ethresear.ch | EIP-7732가 Glamsterdam 헤드라인 EIP로 예정됨. MEV-burn은 포함되지 않음 | https://ethresear.ch/t/epbs-distilled/25800 |
| 3 | Price of Censorship: Censorship Resistance and Throughput under Rational Concurrent Proposers | Saraf, Kaklamanis, Wadhwa, Elsheimy | 2026-07 | arXiv 2607.16995 | 다중 proposer는 검열 비용을 올리지만 중복을 낳음. duplication-penalizing TFM이 최선 | https://arxiv.org/abs/2607.16995 |
| 4 | Properties of issuance offsets and increased penalties under low/zero/negative issuance policies | A. Elowsson | 2026-06 | ethresear.ch | issuance offset과 penalty 강화는 동치로 만들 수 있고, 저·무 issuance에서도 역할 간 상대 인센티브를 보존함 | https://ethresear.ch/t/25292 |
| 5 | SIMD-0550: Double Disinflation Rate (구 SIMD-0411) | Lostin 외 (Helius) | 2026-06 | SIMD, Review | disinflation을 -15%에서 -30%/yr로. terminal 1.5% 도달을 약 5.7년에서 2.8년으로 단축 | https://raw.githubusercontent.com/solana-foundation/solana-improvement-documents/main/proposals/0550-double-disinflation.md |
| 6 | Perils of Parallelism: TFMs under Execution Uncertainty | Wadhwa, Yaish, Zhang, Nayak | 2026-04 | USENIX Security '26 | 병렬 실행에서는 사용자 보호와 스케줄러 보호가 근본적으로 충돌함(불가능성) | https://arxiv.org/abs/2604.04193 |
| 7 | Blockspace Under Pressure: Spam MEV on High-Throughput Blockchains | Wang, Saraf, Heimbach, Babel, Zhang | 2026-03 | arXiv 2604.00234 | 추가 용량의 점점 큰 몫을 spam이 차지함. 전원 포함 전에 용량을 제한하면 spam이 줄어듦 | https://arxiv.org/abs/2604.00234 |
| 8 | Beyond Winner-Take-All Procurement Auctions | Garimidi, Neuder, Roughgarden | 2026-03 | arXiv 2603.27779 | 계산 작업 조달에서 집중을 억제하는 DSIC 할당(곡률 α), Sybil-proof Tullock 변형, 비례 지불(ex-post safe) | https://arxiv.org/abs/2603.27779 |
| 9 | Application of the Federal Securities Laws to Certain Types of Crypto Assets… (Rel. 33-11412) | SEC (+CFTC guidance) | 2026-03 | SEC interpretation, 2026-03-23 발효 | 5분류 taxonomy. protocol mining/staking은 증권 아님. 무대가 airdrop은 investment of money 요건 불충족 | https://www.sec.gov/files/rules/interp/2026/33-11412.pdf |
| 10 | Regulation Crypto Assets (Proposed, Rel. 33-11434) | SEC | 2026 (월 unverified) | Proposed rule | startup/fundraising exemption, investment contract safe harbor, **airdrop safe harbor** 제안 | https://www.sec.gov/files/rules/proposed/2026/33-11434.pdf |
| 11 | Measuring Per-Opcode Proving Time | (ethresear.ch) | 2026-01 | ethresear.ch | opcode·precompile별 증명 시간을 실측. zk cycle이 실제 증명 시간의 대리지표로 쓸 만한지 검증 | https://ethresear.ch/t/measuring-per-opcode-proving-time/23955 |
| 12 | Repricings for block proving (Part 1, 2) | EF zkEVM team | 2025-12 / 2026-01 | EF blog | 필요 증명 처리량(gas/s) = gas limit ÷ 최대 증명 시간을 기준으로 repricing | https://zkevm.ethereum.foundation/blog/repricings-for-block-proving-part-2 |
| 13 | Characterizing Off-Chain Influence Proof TFMs | Ganesh, Thomas, Weinberg | 2025-12 | arXiv 2512.02354 | OffCIP ⇔ "burn identity". 비암호적 구현에는 무한 공급이나 prior 의존이 필요 | https://arxiv.org/abs/2512.02354 |
| 14 | SIMD-0357: Alpenglow Validator Admission Ticket | wen-coding 외 | 2025-12 | SIMD | epoch당 VAT 1.6 SOL을 vote account에서 차감해 소각, 상위 2,000 stake만 활성 | https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0357-alpenglow_validator_admission_ticket.md |
| 15 | A Small Collusion is All You Need | Y. Gafni | 2025-10 | arXiv 2510.05986 | (일관된 tie-breaking 하에서) 2인 담합에 취약한 메커니즘 집합 = 대규모 담합에 취약한 집합 | https://arxiv.org/abs/2510.05986 |
| 16 | Unaligned Incentives: Pricing Attacks Against Blockchain Rollups | Chaliasos, Swann, Pilehchiha, Mohnblatt, Livshits, Kattis | 2025-09 | arXiv 2509.17126 | DA saturation(<2 ETH, 최대 30분 장애)과 prover-killer(finality 약 94배 지연). 대책은 다차원 TFM | https://arxiv.org/abs/2509.17126 |
| 17 | EIP-8011: Multidimensional Gas Metering | (EIP) | 2025-08 | EIP Draft | tx는 합계 gas로 지불하고, 블록 한도와 base fee 갱신은 bottleneck 차원의 max 기준 | https://eips.ethereum.org/EIPS/eip-8011 |
| 18 | On Ethereum Prover Market Design | M. Bahrani, M. Neuder | 2025-08 | ethresear.ch | proof lottery와 staked allocation 비교. B < V/ln V이면 lottery가 우월 | https://ethresear.ch/t/on-ethereum-prover-market-design/22916 |
| 19 | Statement on Certain Liquid Staking Activities | SEC CorpFin | 2025-08 | Staff statement | liquid staking과 staking receipt token은 증권 거래 아님(조건부) | https://www.sec.gov/newsroom/speeches-statements/corpfin-certain-liquid-staking-activities-080525 |
| 20 | EIP-7999: Unified multidimensional fee market | Elowsson, Buterin, Silva | 2025-08 | EIP Draft | 단일 `max_fee`로 모든 차원을 지불. 4844식 excess 갱신, 정규화, reserve price 일반화 | https://eips.ethereum.org/EIPS/eip-7999 |
| 21 | SIMD-0326: Alpenglow | Solana (Kniep, Sliwinski, Wattenhofer 외, unverified) | 2025-07/08 | SIMD, 거버넌스 통과 | 투표는 off-chain BLS 집계. 투표자별 R·T/2 보상. **epoch 보상 0이면 활성 집합에서 제거**. VAT 전액 소각 | https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0326-alpenglow.md |
| 22 | Shipping an L1 zkEVM #1: Realtime Proving | Ethereum Foundation | 2025-07 | EF blog | 실시간 증명의 표준 정의(지연·비용·전력) 제시 | https://blog.ethereum.org/2025/07/10/realtime-proving |
| 23 | Statement on Certain Protocol Staking Activities | SEC CorpFin | 2025-05 | Staff statement | solo/위임/custodial protocol staking은 "administrative or ministerial"이므로 증권 아님 | https://www.sec.gov/newsroom/speeches-statements/statement-certain-protocol-staking-activities-052925 |
| 24 | Update on Proving Coordination | Aztec Labs | 2025-05 | Aztec forum | bond와 견적 방식 폐기. 무허가 다중 제출, 부분 epoch 증명, 보상 균등 분할, 연속성 가중 | https://forum.aztec.network/t/update-on-proving-coordination/7938 |
| 25 | TFM Design for Leaderless Blockchain Protocols | Garimidi, Heimbach, Roughgarden | 2025-05 | FC 2025 | FPA-EQ(first-price + 균등 분배)는 strongly BPIC이고 최적 welfare의 63.2% 이상. 최적성과 강한 IC는 양립 불가 | https://arxiv.org/abs/2505.17885 |
| 26 | Multiple Proposer TFM Design: Robust Incentives Against Censorship and Bribery | Stouka, Ma, Thiery | 2025-05 | arXiv 2505.13751 | bribery 하 검열 저항 TFM(FOCIL 적용) | https://arxiv.org/abs/2505.13751 |
| 27 | The Hunt: Tracking Organic Prover Killer blocks | EF RIG | 2025-05 | ethresear.ch / rig.ethereum.org | 실제 mainnet 블록 중 gas 대비 증명 시간이 과도한 블록을 식별 | https://ethresear.ch/t/the-hunt-tracking-organic-prover-killer-blocks-on-ethereum/22332 |
| 28 | Prover Killers Killer: You Build it, You Prove it | (ethresear.ch) | 2025-05 | ethresear.ch | delayed execution에서 증명 책임을 builder에게 부여 | https://ethresear.ch/t/prover-killers-killer-you-build-it-you-prove-it/22308 |
| 29 | EIP-7918: Blob base fee bounded by execution cost | Elowsson, Adams, D'Amato | 2025-03 | EIP **Final** | blob base fee ≥ `8192·base_fee/GAS_PER_BLOB` reserve. 1 wei로의 붕괴 방지 | https://eips.ethereum.org/EIPS/eip-7918 |
| 30 | SIMD-0228 투표 결과 | Solana 거버넌스 | 2025-03 | 거버넌스 | 찬성 다수였으나 66.67% 미달로 **부결**. stake 74% 이상, 910 validator 참여 | https://solanafloor.com/news/solana-s-failed-simd-0228-vote-still-haunts-the-network |
| 31 | Towards a Formal Framework of the Ethereum Staking Market | Beccuti 외 | 2025-03 | arXiv 2503.14385 | issuance를 줄이면 solo staker가 기관에 밀려남 | https://arxiv.org/abs/2503.14385 |
| 32 | The Early Days of the Ethereum Blob Fee Market and Lessons Learnt | Heimbach, Milionis | 2025-02 | arXiv 2502.12966 | 블록 패킹 비효율로 최대 70%의 상대 수수료 손실 | https://arxiv.org/abs/2502.12966 |
| 33 | Transaction Fee Market Design for Parallel Execution | Acilan, Constantinescu, Heimbach, Wattenhofer | 2025-02 | arXiv 2502.11964 | 병렬화 부하별로 gas를 차등 부과 | https://arxiv.org/abs/2502.11964 |
| 34 | Incentive-Compatible Collusion-Resistance via Posted Prices | Ferreira, Gafni, Resnick | 2024-12 | arXiv 2412.20853 | 단일 입찰자에서 담합 저항과 IC를 동시에 만족하는 메커니즘은 **정확히 posted-price** | https://arxiv.org/abs/2412.20853 |
| 35 | Resonance: Transaction Fees for Heterogeneous Computation | Bahrani, Durvasula | 2024-11 | arXiv 2411.11789 | broker 경쟁으로 개별화 가격, 효율, budget balance 달성 | https://arxiv.org/abs/2411.11789 |
| 36 | Pricing Factors and TFMs for Scalability-Focused ZK-Rollups | Chaliasos, Mohnblatt, Kattis, Livshits | 2024-10 | arXiv 2410.13277 | ZK-rollup TFM은 sequencing, DA, proving 비용을 분리해 반영해야 함 | https://arxiv.org/abs/2410.13277 |
| 37 | Revisiting the Primitives of TFM Design | Ganesh, Thomas, Weinberg | 2024-10 | arXiv 2410.07566 | "off-chain influence proofness" 정의. **EIP-1559는 이를 만족하지 못함** | https://arxiv.org/abs/2410.07566 |
| 38 | Practical endgame on issuance policy | A. Elowsson | 2024-10 | ethresear.ch | stake 증가를 멈추되 성실한 소규모 solo staker에게는 양의 보상 보장 | https://ethresear.ch/t/practical-endgame-on-issuance-policy/20747 |
| 39 | Proofφ: A ZKP Market Mechanism | Wang, Zhou, Yaish, Zhang, Fisch, Livshits | 2024-04 | arXiv 2404.06495 | auction 기반 ZKP 시장. 사용자와 prover 양측 IC, budget balance | https://arxiv.org/abs/2404.06495 |
| 40 | Foundations of minimum viable issuance | A. Elowsson | 2024-04 | EF RIG | 과도한 issuance는 사회적 비용. yield를 낮춰도 보안이 충분하면 낮추는 편이 모두에게 이득 | https://rig.ethereum.org/post/foundations-of-minimum-viable-issuance |
| 41 | Collusion-Resilience in TFM Design | Chung, Roughgarden, Shi | 2024-02 | arXiv 2402.09321 (venue unverified) | 경합이 있으면 무작위 TFM도 UIC, MIC, OCA-proof를 동시에 만족할 수 없음 | https://arxiv.org/abs/2402.09321 |
| 42 | Barriers to Collusion-resistant TFMs | Gafni, Yaish | 2024-02 | arXiv 2402.08564 | 결정적 메커니즘은 trivial한 경우만 세 성질을 동시 만족 | https://arxiv.org/abs/2402.08564 |
| 43 | Boundless ZK Mining (PoVW) / Proof lifecycle | Boundless docs | n.d. (2025–26) | docs | epoch 약 2일. 보상 75%는 mining(작업 비례), 25%는 staking. 상한 `stake/15`. 시장은 reverse Dutch auction + lock collateral | https://docs.boundless.network/zkc/mining/overview |
| 44 | Succinct Prover Network: Proof Contests | Succinct docs | n.d. | docs | stake 자격 → 최저가 역경매. base fee와 PGU 단가, 미이행 시 slash, protocol/staker/owner로 분할 | https://docs.succinct.xyz/docs/protocol/spn/auction |
| 45 | Foundations of TFM Design | Chung, Shi | 2021 | SODA 2023 | UIC, MIC, SCP 동시 만족 불가(경합 시) | https://arxiv.org/abs/2111.03151 |
| 46 | Transaction Fee Mechanism Design | T. Roughgarden | 2021 | EC'21 (arXiv 2106.01340) | EIP-1559는 MMIC와 OCA-proof를 만족. 수요 급증 시 외에는 UIC | https://arxiv.org/abs/2106.01340 |
| 47 | Dynamic Pricing for Non-fungible Resources (Multidimensional fee markets) | Diamandis, Evans, Chitra, Angeris | 2022 | arXiv 2208.07919 (venue unverified) | 자원별 가격을 쌍대 최적화로 갱신하면 네트워크 목표와 사용자 후생이 정렬됨 | https://arxiv.org/abs/2208.07919 |
| 48 | 디지털자산기본법 입법 동향 | 이데일리, 아시아경제, 연합뉴스 | 2026-07~09 | 언론 | 미통과. 10개 법안 계류, 공청회(9/30 추진), 11월 법안소위, 쟁점은 거래소 대주주 지분과 은행 50%+1 | https://www.asiae.co.kr/article/2026092216564554891 |

---

## 3. 쟁점별 결론

### 3.1 TFM (base fee, tip, 소각)
- **불가능성은 확정된 상태입니다.** 경합이 있으면 UIC, MIC, 담합(OCA/SCP) 방지를 동시에 만족할 수 없습니다(#41, #42, #45). EIP-1559조차 OffCIP를 만족하지 못합니다(#37). 가능한 경우는 burn identity로 특성화됩니다(#13).
- **소수 committee 체인은 담합이 가장 쉬운 환경입니다.** 2인 담합이면 충분합니다(#15). 담합 저항과 IC를 함께 얻는 유일한 형태는 posted price입니다(#34).
  → **Aether의 목표는 블록이 거의 차지 않는 영역에서 base fee가 사실상 posted price로 동작하게 하는 것입니다.** 여기에 base fee 소각을 더하면 proposer가 base fee 수입을 담합으로 되찾는 경로가 사라집니다(#46).
- **Tip은 proposer에게 100% 주면 안 됩니다.** Solana SIMD-0096(9월 보고서)에서 보듯 가짜 tip 비용이 0이 됩니다. Ethereum도 2026-08에 self-build를 포함한 tip 소각을 제안했습니다(#1).
- **1초 블록에는 EIP-1559의 블록당 ±12.5%가 과격합니다.** 4844/7999식 지수형 excess 갱신에 작은 갱신폭을 쓰는 편이 좋습니다(#20).
- **Reserve price(floor)가 필요합니다.** fee가 1 wei로 붕괴하는 것을 막고(#29), spam 억제에도 필요합니다(#7).
- **병렬 실행 TFM(#6, #33)은 참고만 합니다.** Aether의 BAL 기반 병렬 실행은 결과를 순차 실행과 동일하게 보장하므로 과금을 순차 기준으로 두면 됩니다. 해당 불가능성은 "실행 불확실성을 과금에 반영할 때" 생기는 문제입니다.

### 3.2 Prover 지급
- **Proving 비용은 EVM gas와 상관관계가 약합니다.** prover-killer(#16, #27, #11, #12)가 그 증거입니다. 그래서 **prove 차원에 독립 base fee와 블록 한도**를 둬야 합니다(#16, #20, #36).
- **"블록 유효성에 증명이 필요하고, proposer가 사서 붙인다"(Mina, 9월 옵션 A)는 증명이 즉시 붙는 체인에서만 성립합니다.** 증명이 수 분~수 시간 뒤따라오는 Aether에서는 **블록 확정 시 수수료를 escrow에 적립하고, 증명이 제출되면 지급**해야 합니다. Aztec이 2025년에 전환한 모델이 이것입니다(#24).
- **담보(bond) 문제:** 가치 0인 토큰으로 받는 bond는 억지력이 없습니다. 담보가 작으면 lottery나 개방형 모델이 staked allocation보다 낫습니다(#18). 이중 작업은 "짧은 예약 창(reservation)"으로 줄입니다. Boundless의 lock, Succinct의 assignment와 같은 역할입니다(#43, #44).
- **집중 억제:** winner-take-all 조달은 prover 집중을 부릅니다(#8). 자원봉사 Mac이 다수이므로 **동시 예약 상한과 chunk 단위 분할**로 분산합니다. Boundless의 `stake/15` 상한(#43)을 reputation 기반으로 바꾼 형태입니다.
- **첫 유효 증명에 지급합니다.** 더 긴 범위의 증명이 짧은 증명을 무효화하는 Aztec식 "superseded" 규칙은 volunteer의 작업을 버리게 하므로 채택하지 않습니다(#24 참고).

### 3.3 Issuance
- Ethereum은 MVI와 reward curve 논의(#38, #40)가 2026에도 진행 중이며(#4), 아직 확정되지 않았습니다. Solana는 동적 곡선(SIMD-0228)이 부결됐고(#30) 단순한 disinflation 강화(#5)가 검토 중입니다. Issuance를 줄이면 소규모 staker가 밀려나는 효과가 있습니다(#31).
- 이 논쟁들은 모두 **토큰에 가치가 있고 stake가 보안을 사는 상황**을 전제로 합니다. Aether는 가치 0이고 committee가 허가형 threshold BLS이므로 issuance가 사는 보안이 없습니다. 반면 "보상 약속"이라는 법적 표면적은 생깁니다.
  → **Issuance 0을 유지합니다.** 나중에 도입하더라도 Elowsson(#4)의 "offset/penalty" 틀로 역할 간 상대 인센티브만 설계하면 됩니다.

### 3.4 Points와 법률
- 미국: 무대가 airdrop은 investment of money 요건을 충족하지 않고, protocol mining과 staking은 증권이 아닙니다(#9, #23, #19). 다만 SEC는 **"명시적 약속(representations or promises)"이 investment contract를 만든다**고 강조했습니다(#9, Atkins 2026-03-17 연설). 그래서 **전환이나 가치를 암시하는 문구는 금지**합니다. Airdrop safe harbor는 아직 제안 단계입니다(#10).
- 한국: 디지털자산기본법은 미통과입니다(#48). 가상자산 과세는 2027-01-01 시행 예정이지만(22%, 기본공제 250만원) 유예·폐지 논쟁이 있습니다(#48, 이데일리 2026-09-21). **비양도·비교환 points는 가상자산 정의(특금법 등)에 해당할 가능성이 낮을 것으로 보이나 (unverified, 자문 필요).**
- Sybil: 행동 기반 탐지(ML)는 사후 수단일 뿐입니다(arXiv 2607.27370, 2505.09313). 설계 단계의 원칙은 두 가지입니다. **검증 가능하고, 실제로 수요된 작업만 계상**하고, **수수료 액수 기반 점수는 금지**합니다(가짜 fee로 부풀릴 수 있음).

### 3.5 소수 committee 특수성
- 보상이 아니라 **"참여하지 않으면 제외"** 로 강제합니다. Alpenglow의 zero-reward removal과 VAT가 그 예입니다(#14, #21). FOCIL의 무보수 1-of-N 의무(9월 보고서)와 같은 계열입니다.
- 보상이 없을 때도 상대 인센티브는 penalty로 보존할 수 있습니다(#4).
- 다중 proposer, 균등 분배(FPA-EQ, #25; #3; #26)는 Aether가 단일 leader이므로 당장은 해당하지 않습니다. 장래에 검열 저항이 필요해지면 검토합니다.

---

## 4. Aether 적용 권고: 지금 코드로 구현할 사양 (v0, zero-value testnet)

표기: 1 block ≈ 1 s. `GasVector{exec,state,prove}`, `limits`는 기존 `BlockContext.limits`입니다. 모든 상수는 genesis config에 두어 조정 가능하게 합니다. 모든 금액은 u128/U256 wei 정수 연산입니다.

### R1. Base fee: 차원별 지수형 갱신 (EIP-4844/7999식)
```
for d in {exec, prove, da}:                      # state는 hook만 둠(현재 0)
  target_d  = limits_d / 2
  excess_d' = max(0, excess_d + used_d(parent) - target_d)       # header에 저장
  base_d    = max(FLOOR_d, fake_exponential(FLOOR_d, excess_d', target_d * K))
K = 96   # 한도까지 찬 블록 1개 ⇒ +1.05%, 빈 블록 ⇒ -1.04%; 12블록(12s) ≈ ±13% (Ethereum 12s 슬롯과 같은 속도)
FLOOR_exec  = 1 gwei       FLOOR_prove = 1 gwei (prove-gas 단위)       FLOOR_da = 1 gwei/byte
reserve (EIP-7918 일반화): base_prove ≥ base_exec / 4,  base_da ≥ 16 · base_exec   (초기값, 측정 후 조정)
```
- 트랜잭션은 **단일 `max_fee`**(EIP-7999)와 `max_priority_fee_per_gas`(exec gas 기준)를 지정합니다. 포함 조건은 `max_fee ≥ Σ_d base_d·limit_d + tip`이고 잔액은 환불합니다. 과거 1559 tx는 `max_fee = max_fee_per_gas·gas_limit`로 변환합니다.
- `limits.prove`를 **블록 유효성 한도**로 강제해 prover-killer 블록을 막습니다(#16, #27). 초기값은 "Mac 1대가 chunk 1개를 T_chunk 안에 증명할 수 있는 prove-gas × chunk 수 상한"에서 역산하되, S5 스파이크에서 opcode별 zk cycle 가중치가 나오면 `ProveGasMeter`를 교체합니다(#11, #12).
- **소각 규칙:** `base_exec`는 100% 소각합니다(#46, #13, #34). `base_prove`는 **100% prover escrow**로 보내며 소각하지 않습니다(R3). `base_da`는 DA reserve로 보냅니다(R6). proposer가 base 수입을 받지 않는다는 핵심 성질은 셋 모두 유지됩니다. prove/da 몫이 proposer가 아닌 제3자에게 가므로 OCA 논리가 그대로 성립합니다(#46).
- 근거: #20(단일 max_fee와 4844식 통합 갱신), #29(reserve price), #7(spam 억제를 위한 floor), #16·#36(proving과 DA를 분리 가격화), #34(비혼잡 영역의 posted-price가 담합 저항).

### R2. Priority fee(tip) 분할
```
tip_total = Σ_tx min(max_priority_fee_per_gas, (max_fee - Σ base·limit)/gas_used_exec) * gas_used_exec
proposer  : 60%   → leader_address (현행 beneficiary)
prover    : 20%   → ProverEscrow[height] (R3)
burn floor: 20%   → 소각 (self-build 포함 무조건)
```
- 20% burn floor는 proposer가 자기 tx로 tip을 부풀리는 비용을 tip의 20%로 만듭니다(SIMD-0096 교훈, #1). 비율은 EIP-8375의 "fraction TBD"에 맞춰 상수로 둡니다.
- Prover 20%의 근거: floor 영역에서는 base fee가 고정이라 수요 증가가 tip으로만 드러나고, Aether의 병목은 증명 처리량(Mac 1대당 ~270 tx/h)입니다. 혼잡 신호를 병목 자원 공급자에게 일부 전달하려는 것입니다(#16, #36).
- **중요:** tip 분할은 **fee credit 단계**에서 한 번에 계산합니다. `touched_beneficiary` 재실행 로직(`block.rs:162`)과 충돌하지 않도록 proposer 크레딧은 블록 끝의 단일 state write로 처리합니다.

### R3. Prover pool: 증명이 뒤따라와도 되는 escrow와 사후 청구
**적립(블록 확정 시, 결정적):**
```
ProverEscrow[h] = Σ base_prove·prove_used(h) + 0.20·tip_total(h)
chunk 정의: 블록 h의 tx 구간 [i,j) 또는 블록 구간 [h1,h2]
ChunkCommitment C = H(chain_id, h1, h2, i, j, pre_state_root, post_state_root, Σprove_gas, guest_program_id)
payout(C) = Σ_{h∈C} ProverEscrow[h] · prove_gas(C∩h) / prove_gas(h)
```
**청구(증명 제출 시):**
1. 시스템 tx `SubmitChunkProof(C, proof, prover_id)`를 보냅니다. committee는 네이티브로 Jolt 검증을 수행하고, 통과하면 블록에 `ChunkProven{C, prover_id, prove_gas, cycles, latency}`를 기록합니다.
2. **C당 첫 번째 유효 증명만 지급합니다**(Aztec #24, Taiko 계열). 이미 증명된 C를 포함하는 aggregate/checkpoint 증명은 **미청구 chunk만** 받습니다. 기존 청구를 무효화(supersede)하지 않습니다. aggregator에게는 별도 5% 수수료를 주며 escrow에서 선공제합니다.
3. `Claimable[prover_id] += payout(C)`로 쌓고, pull 방식 `ClaimProverRewards()`로 인출합니다. 언제든 나중에 인출할 수 있습니다(Boundless의 미청구 누적 인출, #43).
4. **예약(선택):** `ReserveChunk(C)`는 예약 창 `W = 2h` 동안 해당 prover만 지급 대상이 되게 합니다. 동시 예약 상한은 `1 + floor(proven_chunks_90d / 50)`, 최대 16입니다(#8 집중 억제, #43 상한의 reputation판). 창이 만료되면 개방되며 누구나 첫 유효 증명을 제출할 수 있습니다. 미이행은 `ChunkMissed` receipt로 기록됩니다(평판 penalty, 토큰 slash 없음, #18).
5. **만료:** `EXPIRY = 2,592,000 blocks (~30일)` 동안 증명되지 않은 escrow는 **소각**합니다. proposer 환급은 금지합니다. 환급하면 proposer에게 증명을 방해할 유인이 생깁니다. (구현, 13-protocol-2.md §3: 만료분은 `PROVER_ESCROW` 잔고에 그대로 남고 누구에게도 가지 않습니다. 사실상 소각이며, 따로 소각 처리를 하지 않습니다.)
- 체크포인트만 증명하는 초기 단계에서도 체크포인트 증명이 구간 전체의 escrow를 받습니다. 규칙 2에 따라 먼저 증명된 chunk는 제외합니다.
- **v1 확장:** `excess_prove`에 **미증명 backlog**(= 확정된 prove_gas − 증명된 prove_gas, 온체인에서 결정적으로 계산 가능)를 반영합니다. 허용 지연 `B0 = 1h·target`을 넘으면 `base_prove`가 오르게 하는 backlog 제어입니다.

### R4. Committee 활동성
- **보상 0입니다(issuance 없음, committee 전용 fee 몫 없음).** proposer 몫 60%는 leader가 순환하므로 committee 전체에 자연스럽게 분배됩니다.
- **참여 기록:** 각 블록에 finalization certificate 서명자 bitmap(`signers: u64`)을 기록합니다. Alpenglow aggregates와 같은 역할입니다(#21).
- **제거 규칙(Alpenglow zero-reward removal의 무보상판):** `EPOCH = 3,600 blocks (1h)`로 둡니다.
  - 한 epoch에서 서명 참여율이 **0%이거나 leader 차례 누락률이 50%를 넘으면** `Inactive` 플래그를 기록합니다.
  - **2 epoch 연속** `Inactive`이면 다음 epoch 경계에서 committee 제외 대상이 됩니다.
  - threshold BLS reshare(DKG)가 구현되기 전까지는 제외 대상을 운영자 재구성 절차로 반영하고, 그 사실을 온체인 receipt로 남깁니다.
- VAT(#14)는 가치 0 토큰에서는 의미가 없으므로 넣지 않습니다. 구조만 hook으로 남깁니다(`admission_fee = 0`, 소각 대상).
- 근거: #21, #14(제거로 활동성 강제), #4(무보상에서 penalty로 상대 인센티브 보존), 9월 보고서의 FOCIL 무보수 의무.

### R5. Points / work receipts (비양도)
블록마다 시스템 영역(`receipts` 또는 system contract storage)에 다음을 기록합니다.

| 역할 | 기록 필드 | 계상 기준 |
|---|---|---|
| Proposer | `height, leader_id, tx_count, exec_used, prove_used` | **블록 수 기준**, fee 액수는 금지(가짜 tip 방지) |
| Committee | `signers bitmap` | epoch별 참여율 |
| Prover | `ChunkProven{C, prover_id, prove_gas, zk_cycles, latency_blocks}`, `ChunkMissed{C, prover_id}` | **첫 유효 증명의 prove_gas**. 실제 체인 블록(수요된 작업)에 대해서만 인정 |
| Aggregator | `AggregateProven{range, prover_id}` | 건수 |

- **양도 불가:** transfer, approve, ERC-20/721 인터페이스를 두지 않습니다. `prover_id`는 P-256 Secure Enclave 키(기존 지갑 인프라)에 묶습니다. 기기 키를 기기 1대당 Sybil 비용으로 쓰는 방안은 **App Attest 적용 가능 여부 unverified**입니다.
- 점수 함수(가중, 감쇠, 상한)는 **오프체인 표시용**입니다. 프로토콜 내 용도는 R3-4의 예약 상한과 향후 committee 선발 가중치뿐입니다.
- 근거: #43(작업 증명 기반 계량), #8(Sybil-proof 조달), #9·#10(명시적 약속 금지, 무대가 배포), 9월 보고서의 Helium/Filecoin 교훈.

### R6. DA fee hook — 보류

미구현. `fees.rs`는 exec·prove 차원만 처리하고 `da` 차원과 `DA_RESERVE`는 없습니다. 외부 DA(00-overview D11)와 함께 보류합니다.

- 차원 `da`의 사용량은 `used_da = Σ tx 직렬화 바이트`입니다. `base_da`는 R1 규칙을 따르고 reserve는 `base_da ≥ 16·base_exec`입니다(EIP-7918 일반화, #29).
- 수입은 `DA_RESERVE` 시스템 계정으로 보냅니다. 외부 DA(Celestia) 게시가 켜지기 전에는 **매 블록 소각**하고, 켜진 뒤에는 게시 비용 정산에 씁니다. 이는 DA saturation 공격(#16)에 대한 가격 방어도 겸합니다.

### R7. 하지 말 것
1. **Issuance를 두지 않습니다.** block reward, PoVW emission, staking yield 모두 해당합니다(3.3, #9·#10 법적 표면적).
2. **양도 가능한 points, 토큰 전환 약속, "에어드랍 예정" 문구를 쓰지 않습니다**(#9 "명시적 약속"이 investment contract를 형성, #48 국내 법 미확정).
3. **fee 액수에 비례하는 점수나 보상을 주지 않습니다**(가짜 tip farming, SIMD-0096).
4. **base_exec를 proposer나 committee에게 주지 않습니다**(#46, #34, #15).
5. **미증명 escrow를 proposer에게 환급하지 않고, 기존 chunk 청구를 supersede하지 않습니다**(R3, #24).
6. **가치 0 토큰으로 bond나 slash를 하지 않습니다.** 억지력이 없고 복잡도만 늘어납니다(#18).
7. **MEV 경매나 builder 시장을 도입하지 않습니다.** 1s 블록 소수 committee에서는 담합 표면만 늘어납니다(#2, #15).
8. **블록 유효성을 증명에 의존시키지 않습니다**(증명은 뒤따라옴. Mina식 강제는 부적합).

### 구현 순서 (작은 PR 단위)
1. `BlockHeader`에 `excess: GasVector`와 `base_fee: GasVector` 추가, `fake_exponential` 구현, `b.basefee = base_exec`. 테스트: 한도까지 찬 블록 / 빈 블록 / floor.
2. Fee 정산 모듈 `fees.rs`: burn, proposer, escrow, DA 분기 (R1, R2, R6). 테스트: 총량 보존(`paid = burned + proposer + escrow + da`).
3. `ProverEscrow`, `ChunkCommitment`, `SubmitChunkProof`, `Claim` (R3).
4. Receipts와 signer bitmap, `Inactive` 판정 (R4, R5).

---

## 5. 미해결 / 후속
- **prove-gas 가중치:** 현재 명령어당 1 unit입니다. S5에서 Jolt의 opcode·precompile별 cycle(P-256, BLAKE3 inline 반영)을 측정해 교체해야 합니다(#11, #12). 그 전까지 `limits.prove`는 보수적으로 둡니다.
- **Chunk 경계 표준화:** tx 구간 chunk와 블록 구간 chunk를 섞으면 payout 비례 계산에 필요한 per-tx prove_gas를 receipts에 기록해야 합니다(이미 `Receipt.prove_gas` 존재).
- **네이티브 Jolt 검증 비용:** committee가 검증할 때의 시간 예산과, 검증을 샘플링해도 되는지 확인이 필요합니다.
- **Committee 제외의 DKG reshare 구현:** threshold 키 재분배 전까지 운영 절차가 필요합니다.
- **파라미터 캘리브레이션:** K=96, 60/20/20, reserve 비율, EXPIRY, W를 testnet 데이터로 재조정합니다.
- **법률:** 한국 디지털자산기본법 통과 시 비양도 points의 지위와 과세 여부, 미국 Regulation Crypto 최종안의 airdrop safe harbor 조건을 확인해야 합니다. **토큰 가치 부여나 전환 전 KR·US 자문은 필수입니다.**
- **Unverified 항목:** SIMD-0326 저자 목록, Reg. Crypto 제안 월, Chung–Roughgarden–Shi와 Diamandis 외 venue, App Attest를 Sybil 비용으로 쓸 수 있는지, 비양도 points의 국내 가상자산 해당 여부.

---

## 6. 개정 (2026-09-26): 공정 출시 A+B

사용자 결정: **사전 발행 없이 시작하되, 토큰이 없어도 쓸 수 있고 증명한 Mac이 토큰을 받는다.** R1과 R7(1)을 아래처럼 바꿉니다. 나머지 R1~R6은 그대로 둡니다.

**배경:** R7(1)의 "발행 0"은 가치 0 testnet을 전제로 했습니다. 가치를 붙이면 제네시스에 아무도 토큰이 없고, 수수료도 없고, 검증자·prover 수입도 없습니다. testnet은 faucet 몫을 제네시스에 넣어 돌렸는데, 이를 그대로 쓰면 운영자 사전 발행이 되어 공정하지 않고 법적 위험이 가장 큽니다.

### R1′. 혼잡하지 않으면 수수료 0 (B)
```
base_d = FLOOR_d · (e^(excess_d / (target_d · K)) − 1)      # 기존: max(FLOOR_d, FLOOR_d · e^(...))
```
- 블록이 목표량(한도의 절반) 이하로 차면 `excess = 0`이고 기본 수수료가 **0**입니다. 토큰이 없는 사용자도 쓸 수 있습니다.
- 블록이 목표를 넘겨 계속 차면 수수료가 지수적으로 오릅니다. 스팸이 스스로 비싸지므로 계정별 무료 할당량(Sybil에 취약)이 필요 없습니다.
- 우선순위는 tip으로 삽니다(R2 분할 유지). 수수료 상한 검사는 "상한 ≥ 기본 수수료"이고, 0이면 상한 0도 통과합니다.
- 멤풀의 발신자당 대기 64건 상한과 블록 한도는 그대로 유지합니다.

### R7′(1). 증명 발행 (A)
- **제네시스 잔액 0.** faucet은 testnet에만 둡니다(mainnet 제네시스에 없음).
- 새 토큰은 **유효한 블록 증명**이 확정될 때만 발행되고, 그 블록을 처음 증명한 prover에게 지급됩니다(첫 유효 증명, 30일 만료). 구현은 블록 단위 정액이고, prove gas 비례는 보류입니다(아래 구현 상태).
- 블록당 발행량은 `ISSUE_0 · 2^(−h / HALVING)`입니다. 반감기마다 절반이 되고 결국 0에 가까워집니다(꼬리 발행 없음).
  - 초기값: `ISSUE_0 = 1 AETH/block`, `HALVING = 31,536,000 blocks`(1초 블록 기준 약 1년). 총발행 상한은 약 `2 · ISSUE_0 · HALVING` ≈ 6,300만 AETH입니다.
- 검증자(합의) 몫은 없습니다. 제안자는 tip의 60%를 받고, 운영 비용은 원래 켜 두는 Mac이라 작습니다. 같은 Mac이 prover로 발행을 받는 구조입니다.
- **법적 표면:** 발행은 "보상 약속"이 될 수 있습니다(3.3, #9·#10). 가치 부여 전에 14단계 법률 검토에서 다시 봅니다. 발행 공식은 위원회 서명 업그레이드(5단계)로만 바꿀 수 있습니다.

### 구현 상태
- R1′: `crates/execution/src/fees.rs` (base_fee 공식, 수수료 상한 검사, 멤풀 입장 검사).
- R7′: `crates/execution/src/proofs.rs` (발행 일정 `ISSUE_0`·`HALVING`, 만료 `EXPIRY`, 블록별 기록과 지급). 프로토콜 2로 testnet에서 높이 기반 활성화(13-protocol-2.md).
- 실제 지급 모델은 위 R3의 청크·prove gas 비례가 아니라 **블록 단위**입니다(13-protocol-2.md §3): 블록 h+1이 C(h)와 에스크로 몫 E(h)를 상태에 기록하고, 블록 h의 첫 유효 증명을 담은 블록에서 증명자 주소에 E(h) + 발행(h)을 한 번에 지급합니다. 청구·인출(pull)·예약·aggregator 수수료는 없습니다. 증명되지 않은 블록의 발행분은 생기지 않고, 에스크로는 30일 뒤에도 남아 사실상 소각됩니다.
- `crates/proving/src/market.rs`의 청크 시장 코드(prove gas 비례, pull 청구)는 노드에 연결되어 있지 않습니다.

## 7. 개정 (2026-09-26): 기여자만 받는다, 작업량 × 연속 기여 순위

사용자 결정:
- **faucet 없음(본 네트워크):** R1′로 비혼잡 시 수수료가 0이므로 사용에 토큰이 필요 없습니다. 토큰은 기여한 노드에게만 갑니다. faucet은 testnet 전용입니다.
- **작업량 비례 — 보류(프로토콜 3 후보):** 보상은 증명한 prove gas에 비례합니다(R3/R7′). 큰 Mac이 큰 청크를 증명해 더 받습니다. 청크는 Mac 등급(칩, GPU 코어, 통합 메모리, 측정한 Metal 성능)에 맞춰 배정합니다. 프로토콜 2는 블록 단위로 증명하고 블록마다 정액(E(h) + 발행)을 지급합니다.
- **연속 기여 순위 — 가중은 보류(프로토콜 3 후보):** 노드별 연속 기여 기간 `streak`(에포크 수)를 온체인에 기록합니다(구현: CommitteeRegistry, 투표 후보의 생존 신호. 지금은 추첨 자격에만 씀). 가중치는 `work × (1 + min(streak / S, 1))`, 곧 최대 2배입니다. 증명자 주소가 등록 후보와 연결되어 있지 않아 아직 적용하지 않습니다. 유예 `G`(초기 24시간)를 넘겨 기여가 끊기면 `streak = 0`입니다. 일찍 참여한 노드는 반감기 일정으로 높은 발행을 받고, 오래 유지하면 순위 보너스를 받습니다.
- **신원:** 순위는 등록된 기기에 묶어, 양도·복제·Sybil을 막습니다. macOS는 App Attest를 쓸 수 없어(Developer ID 배포) DeviceCheck로 기기 1대 = 투표 키 1개를 확인합니다(14-registration.md). iPhone 앱은 App Attest.
- 초기값: `S = 90일`(보류, 코드에 없음), `G = 24시간`(구현: `GRACE_EPOCHS = 24`). 모두 위원회 서명 업그레이드로만 바꿉니다.
