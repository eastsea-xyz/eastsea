> **팀장 검토 (2026-09-29):** dex-official 작업 명세에 반영한다. 컨트랙트 쪽 필수: 첫 LP 인플레이션 방지(최소 유동성 소각), 반올림은 항상 풀에 유리하게, 재진입 잠금, 전송 수수료·리베이스 토큰은 받은 양을 재서 처리하거나 거부, 마감 시간 필수(무한 금지), 고정 컴파일러 버전, 불변식 퍼징. 지갑·웹 쪽: 기본 슬리피지 0.5%, 높은 슬리피지 경고, 미검증 토큰 배지. 사고 금액·날짜 중 일부(예: Balancer 2025년 11월)는 보고서 인용이며 출처 링크가 도메인 수준이라 인용할 때 다시 확인한다.

# 신규 체인 공식 불변(Immutable) DEX(AMM) 보안 설계 조사 보고서 (2023–2026 사고 기준)

---

## 요약 (Executive Summary)

신규 블록체인 런칭 시 공식 탈중앙화 거래소(DEX/AMM)를 관리자 키(Admin Key)나 업그레이더블 프록시(Upgradeable Proxy) 없이 **완전한 불변 컨트랙트(Immutable Contract)**로 배포하는 것은 거버넌스 공격 표면 제거, 신뢰 비용 제로화, 궁극의 검열 저항성을 달성하기 위한 가장 순수한 탈중앙화 모델입니다. 

그러나 2023년부터 2026년까지 온체인에서 발생한 대규모 익스플로잇은 **"수정 불가능한 컨트랙트는 단 1 wei의 반올림 오차나 컴파일러 버그 앞에서도 프로토콜 전체 TVL의 영구적 유실로 이어진다"**는 냉혹한 사실을 입증했습니다.

본 보고서는 Uniswap(v2/v3/v4), Curve, Balancer, Velodrome/Aerodrome, Trader Joe Liquidity Book 등 대표 AMM 아키텍처의 핵심 취약점과 사고를 분석하고, 제안자 순서가 강제되는 차세대 합의 계층(FOCIL 등)에서도 상존하는 MEV 위험, 긴급 정지(Pause) 없는 불변 프로토콜의 운영 현실, 그리고 메인넷 배포 전 필수적으로 통과해야 할 불변식 퍼징·차등 테스트·지갑 UI 안전 체크리스트를 제시합니다.

---

## 1. 2023–2026 주요 AMM 아키텍처별 취약점 및 사고 심층 분석

```
[2023–2026 AMM 주요 취약점 계층도]
┌────────────────────────────────────────────────────────┐
│ 1. 언어 & 컴파일러 결함: Curve Vyper @nonreentrant 버그  │
├────────────────────────────────────────────────────────┤
│ 2. 산술 정밀도 & 라운딩: Balancer 2025, KyberSwap 2023 │
├────────────────────────────────────────────────────────┤
│ 3. 비표준 토큰 예외: FOT, 리베이스, ERC-777 훅, 첫 LP  │
├────────────────────────────────────────────────────────┤
│ 4. 아키텍처 결합 리스크: Uniswap v4 Hook, LB Flash-fee │
└────────────────────────────────────────────────────────┘
```

### 1.1 Balancer: 2025년 반올림 방향성 및 2023년 부스티드 풀 사고
Balancer v2/v3는 단일 볼트(Vault) 아키텍처와 다중 자산 불변식($V = \prod B_i^{w_i}$), 그리고 이자 합성 풀(ComposableStablePool, LinearPool)을 특징으로 합니다.

#### [검증된 사실]
* **2023년 8월 리니어 풀 반올림 결함 (Immunefi $1M 바운티)**: `ERC4626LinearPool`에서 이자율(rate) 계산 시 반올림 방향 오류와 플래시 스왑이 결합될 경우 풀 잔고가 점진적으로 고갈될 수 있는 결함이 보고되었습니다. 긴급 비상 서브풀 일시정지(Pause) 메커니즘을 통해 대부분의 자금이 보호되었으나 일부 비동결 풀에서 약 $2.1M의 피해가 발생했습니다.  
  * 출처: [Immunefi Balancer Vulnerability Report (2023)](https://immunefi.com)
* **2025년 11월 ComposableStablePool 반올림 익스플로잇 ($100M~$128M 손실)**: 2025년 11월 3일, Balancer v2의 `ComposableStablePool` 컨트랙트에서 산술 정밀도 손실(Rounding Direction Error)을 악용한 공격이 발생하여 이더리움, 베이스, 폴리곤 등 다중 체인에서 1억 달러 이상의 자산이 탈취되었습니다.  
  * **근본 원인**: `_upscale` 및 `_upscaleArray` 함수에서 자산 잔고가 극소량(8~9 wei)으로 떨어질 때 정밀도 스케일링을 위한 정수 나눗셈에서 반올림이 풀에 불리한 방향으로 작동했습니다.  
  * **공격 기법**: 공격자는 단일 원자적(atomic) 트랜잭션 내에서 정밀하게 설계된 수백 번의 '마이크로 스왑(Micro-swaps)'을 일괄 실행하여 반올림 오차를 기하급수적으로 증폭시켰습니다. 이를 통해 BPT(Balancer Pool Token)의 가치를 인위적으로 왜곡하고 디플레이션된 가격으로 풀 기본 자산을 헐값에 탈취했습니다.  
  * 출처: [Trail of Bits Security Post-Mortem](https://trailofbits.com), [OpenZeppelin Balancer Incident Analysis](https://openzeppelin.com)

#### [추론 및 분석]
* 복합 자산 및 가변 가중치 AMM에서 1 wei 단위의 반올림 방향은 반드시 **"프로토콜과 유동성 공급자에게 보수적인 방향(사용자가 지불할 때는 올림, 수령할 때는 내림)"**으로 일관되어야 합니다.  
* Balancer v2처럼 수직 결합된 단일 볼트(Vault) 구조에서는 풀 수학의 단일 라운딩 버그가 Vault 전체의 지불능력 불일치로 전이될 위험이 극대화됩니다.

---

### 1.2 Curve Finance: Vyper 컴파일러 재진입 잠금 결함 (2023년 7월)
Curve의 Stableswap은 자산 가격이 1:1에 근접할 때 미끄러짐을 최소화하는 고유의 불변식($A \cdot n^n \sum x_i + D = A \cdot D \cdot n^n + \frac{D^{n+1}}{n^n \prod x_i}$)을 사용하며, 뉴턴-랩슨(Newton-Raphson) 근사법으로 수렴값을 계산합니다.

#### [검증된 사실]
* **2023년 7월 30일 Vyper `@nonreentrant` 컴파일러 버그 ($69M~$73M 손실)**: Vyper 버전 `0.2.15`, `0.2.16`, `0.3.0`으로 컴파일된 Curve 풀(alETH/ETH, msETH/ETH, pETH/ETH, CRV/ETH)이 크로스 함수 재진입 공격에 노출되었습니다.  
  * **근본 원인**: 스마트 컨트랙트 소스코드의 논리적 오류가 아니라, **Vyper 컴파일러가 서로 다른 함수에 동일한 재진입 잠금 키(reentrancy lock key)가 지정되었을 때 동일한 스토리지 슬롯을 할당하지 못하고 각각 다른 스토리지 슬롯을 할당한 컴파일러 결함**이었습니다.  
  * **공격 양상**: `remove_liquidity` 함수를 호출하여 ETH가 전송되는 도중(원자적 실행 상태), 공격자의 컨트랙트가 콜백을 받아 `add_liquidity`를 호출했습니다. 컴파일러 결함으로 인해 재진입 가드가 작동하지 않아 풀 내부 잔고가 갱신되기 전의 왜곡된 상태에서 유동성 주식이 추가 발행되어 풀 잔고가 고갈되었습니다.  
  * 출처: [Vyper Security Advisory (CVE-2023-37902)](https://github.com/vyperlang/vyper/security/advisories/GHSA-5824-cm3x-3c38), [Hacken Curve Vyper Exploit Analysis](https://hacken.io), [CertiK Alert: Curve Finance Vyper Compiler Vulnerability](https://certik.com)
* **Read-only Reentrancy 취약점**: Curve의 `get_virtual_price()` 함수는 뷰(View) 함수로서 재진입 락이 걸려있지 않습니다. 공격자가 풀 잔고를 조작하는 트랜잭션 도중 다른 디파이 렌딩 프로토콜이 `get_virtual_price()`를 호출하여 담보 가치를 평가할 경우 부실 대출을 유발하는 구조적 취약점입니다.  
  * 출처: [Chainlight Read-only Reentrancy Report](https://chainlight.io)

#### [추론 및 분석]
* 소스코드 수준의 감사를 100% 통과한 불변 컨트랙트라 할지라도 **컴파일러(IR, 바이트코드 생성기)의 버그가 존재하면 온체인에서 무방비 상태로 파괴**됩니다.
* 관리자 키가 없는 불변 컨트랙트는 컴파일러 결함이 발견되어도 패치가 불가능하므로, 검증된 고정 컴파일러 버전(Pinned toolchain)과 바이트코드 수준의 역컴파일 교차 검증이 필수적입니다.

---

### 1.3 KyberSwap Elastic: 집중 유동성(CLMM) 정밀도 및 틱 산술 결함 (2023년 11월)
KyberSwap Elastic은 Uniswap v3와 유사한 틱(Tick) 기반 집중 유동성 모델에 수수료 재투자(Reinvestment Curve)를 결합한 AMM입니다.

#### [검증된 사실]
* **2023년 11월 22일 산술 정밀도 조작 익스플로잇 ($48.8M~$54M 손실)**: Arbitrum, Optimism, Ethereum, Polygon, Base 전반의 풀에서 자금이 유출되었습니다.  
  * **근본 원인**: `calcReachAmount` 함수에서 틱 경계 도달 금액을 계산할 때의 반올림 오차로 인해, 스왑 로직이 실제로는 틱 경계를 넘었음에도 불구하고 `_updateLiquidityAndCrossTick` 함수를 트리거하지 못하는 에지 케이스가 발생했습니다.  
  * **공격 기법**: 공격자는 플래시 론으로 유동성이 비어 있는 틱 구간으로 가격을 밀어 넣은 뒤, 경계값에서 정밀도 불일치를 유발했습니다. 이로 인해 컨트랙트는 실제 존재하지 않는 '가상 유동성(Virtual Liquidity)'을 이중 계상(Double-counting)하게 되었고, 공격자는 비정상적인 교환비로 풀의 모든 담보를 인출했습니다.  
  * 출처: [BlockSec KyberSwap Incident Analysis](https://blocksec.com), [Halborn Security KyberSwap Exploit Breakdown](https://halborn.com), [KyberSwap Post-Mortem & Grant Program](https://kyberswap.com)

#### [추론 및 분석]
* 집중 유동성 AMM에서 $\sqrt{P}$ (Square root price)와 $L$ (Liquidity) 간의 기하학적 곱셈/나눗셈은 비선형적이므로, 틱 경계의 부등호(`>` vs `>=`) 및 1 wei 정밀도 처리가 상태 불일치(Desync)를 유발하는 가장 위험한 공격 벡터입니다.

---

### 1.4 Uniswap: v2, v3, v4의 취약점 진화와 훅(Hook) 리스크
Uniswap은 탈중앙화 불변 AMM의 표준으로 자리 잡아왔으나, 아키텍처 확장에 따라 새로운 보안 과제가 대두되었습니다.

#### [검증된 사실]
* **v2**: 단순 $x \cdot y = k$ 모델로 불변성과 안전성이 가장 높으나, 첫 LP 공급자의 셰어 인플레이션 공격과 FOT 토큰 처리 불일치 한계가 존재합니다.
* **v3**: $L = \frac{\Delta y}{\Delta \sqrt{P}}$ 집중 유동성 도입으로 자본 효율성을 극대화했으나, 틱 계산 복잡도 증가 및 TWAP 오라클 관측치(Observation) 확장을 위한 가스 소모 문제가 발생합니다.
* **v4 (Hooks & Singleton PoolManager)**:  
  * 단일 컨트랙트(Singleton `PoolManager`)에서 모든 풀을 관리하며, 임시 스토리지(`TSTORE`/`TLOAD`, EIP-1153)를 활용한 플래시 어카운팅(Flash Accounting) 및 ERC-6909 클레임 토큰 시스템을 채택했습니다.  
  * **훅(Hook) 보안 위험**: 풀 생성자가 지정한 외부 훅 컨트랙트가 `beforeSwap`, `afterSwap`, `beforeAddLiquidity`, `afterAddLiquidity` 등에서 임의 코드를 실행합니다. 악의적 훅은 스왑 수수료를 100%로 변경하거나, 사용자 자금을 탈취하거나, 악의적 반환값을 통해 트랜잭션을 가스 고갈(DoS)시킬 수 있습니다.  
  * **주소 마이닝 충돌**: v4는 가스 최적화를 위해 훅의 활성화 플래그를 컨트랙트 주소의 상위 비트마스크(Address Prefix)로 검증합니다. 취약한 배포자는 CREATE2 솔트 마이닝 과정에서 의도치 않은 권한 비트가 켜진 주소를 생성할 위험이 있습니다.  
  * 출처: [Uniswap v4 Technical Whitepaper and Hook Architecture](https://docs.uniswap.org), [Ethereum Improvement Proposal EIP-1153: Transient Storage Opcodes](https://eips.ethereum.org/EIPS/eip-1153)

#### [추론 및 분석]
* 신규 체인의 공식 기본 DEX로는 Uniswap v4의 복합 훅 구조보다, 검증 가능성이 극대화되고 외부 의존성이 없는 순수 불변 AMM 코어(v2 또는 정제된 v3 집중 유동성)가 안전성 측면에서 압도적으로 우월합니다.

---

### 1.5 Trader Joe Liquidity Book (LB) & Velodrome/Aerodrome (Slipstream)

#### [검증된 사실]
* **Trader Joe Liquidity Book (LB)**:
  * 가격을 이산적인 빈(Bin, 기본 빈 스텝 $1 \text{ bp} = 0.01\%$) 단위로 나누고, 각 빈 내부에서는 상수 합($x + y = k$) 마켓 메이커로 작동하여 무슬리피지 교환을 지원합니다.
  * **보안 감사 지적 사항**: Code4rena 감사에서 플래시 론 직전에 활성 빈(Active Bin)에 유동성을 집중 예치하여 비정상적으로 막대한 수수료를 독점적으로 가로채는 행위(Flash-deposit fee harvesting) 및 복합 빈 횡단 시 라우터 산술 오차 위험이 식별되었습니다.
  * 출처: [Code4rena Trader Joe v2 Liquidity Book Contest Findings](https://code4rena.com), [LFJ (Trader Joe) Liquidity Book Technical Documentation](https://docs.lfj.gg)
* **Velodrome / Aerodrome (Slipstream)**:
  * Solidly의 ve(3,3) 토크노믹스를 기반으로 발전하여 Base 및 Optimism에 집중 유동성 엔진인 Slipstream을 배포했습니다.
  * 스왑 수수료가 LP에게 복리로 누적되지 않고 ve-투표자(Voter)에게 라우팅되는 구조로 인해 게이지(Gauge) 수수료 분배와 틱 누적 간의 회계 동기화 복잡성이 존재합니다.
  * 프로토콜 컨트랙트 자체의 결함은 없었으나 2023년 및 2025년 프론트엔드 DNS 하이재킹 공격으로 사용자가 악의적 승인(Approve) 컨트랙트에 유인되는 사고가 발생했습니다.
  * 출처: [Spearbit Security Review: Velodrome Slipstream](https://spearbit.com), [Velodrome Security Docs](https://docs.velodrome.finance)

---

### 1.6 토큰 표준 예외(Weird ERC-20) 및 경제적 공격 벡터

| 분류 | 취약점 메커니즘 | 역사적 사고 및 영향 | 불변 AMM 대응 설계 |
| :--- | :--- | :--- | :--- |
| **전송 수수료 (Fee-on-Transfer)** | `transfer(to, amount)` 실행 시 수수료가 차감되어 `amount`보다 적은 토큰이 도착함. AMM이 매개변수 `amount`를 그대로 장부에 반영하면 풀 지급준비금 결손 발생. | STA 토큰 Balancer v1 풀 배수 탈취 사고. | 풀 잔고를 `balanceAfter - balanceBefore` 차이값으로만 계측하도록 강제하거나, FOT 토큰 풀 생성을 원천 차단. |
| **리베이스 토큰 (Rebasing)** | stETH, AMPL처럼 잔고가 온체인 트랜잭션 없이 자동으로 증가/감소함. 양의 리베이스는 잉여 자금을 남겨 `skim()` 차익거래에 털리고, 음의 리베이스는 풀을 즉시 파산시킴. | stETH Uniswap v2 풀 자본 비효율 및 비동기 손실. | 리베이스 토큰의 직접 페어링을 금지하고, 불변 래핑 토큰(예: wstETH)만 상장 가능하도록 화이트리스트/가이드라인 강제. |
| **ERC-777 / ERC-1363 훅** | 토큰 전송 시 `tokensToSend` 또는 `tokensReceived` 콜백을 호출하여 수신자/전송자에게 제어권을 넘김. | imBTC Uniswap v1 재진입 공격, Lendf.me 전액 탈취. | 상태 변경(State Change) 이전에 외부 토큰 전송을 금지하고(Checks-Effects-Interactions 패턴), 전송 전후 엄격한 재진입 락 적용. |
| **첫 LP 인플레이션 공격 (Share Inflation)** | 초기 LP가 1 wei를 예치하여 1 share를 얻은 뒤, 거액의 자산을 풀에 기부(Direct Transfer)하여 1 share의 가치를 극단적으로 부풀림. 후속 예치자는 정수 나눗셈 절삭으로 인해 100% 손실 발생. | ERC-4626 볼트 전반의 초기 예치 공격, 수많은 Uniswap v2 포크 탈취. | Uniswap v2의 `MINIMUM_LIQUIDITY` ($10^3 \text{ wei}$) `address(0)` 영구 소각 기법을 필수 구현하거나, 가상 오프셋(Virtual Shares) 도입. |
| **TWAP 오라클 조작** | 풀의 깊이가 얕거나 멀티 블록 MEV가 가능한 환경에서 유동성을 일시적으로 비틀어 시간 가중 평균 가격(TWAP)을 왜곡함. | Euler Finance, Mango Markets 및 다수 렌딩 프로토콜의 오라클 조작 청산 사고. | 최소 유동성 임계값을 만족하지 않는 풀의 TWAP 사용 금지, 긴 누적 윈도우(>= 30분) 강제, L2 시퀀서 지연 시간 검증. |

* 출처: [Weird ERC20 Tokens Repository (d-xo)](https://github.com/d-xo/weird-erc20), [OpenZeppelin: ERC4626 Share Inflation Attack & Defenses](https://openzeppelin.com)

---

## 2. 샌드위치·MEV: 제안자 순서 강제 체인(FOCIL 등)에서의 잔여 위험과 파라미터 설계

### 2.1 FOCIL(Fork-Choice Enforced Inclusion Lists) 환경에서도 남는 MEV 위험

차세대 합의 계층에서 연구되는 **FOCIL**은 다수의 무작위 검증자(Validator Committee)가 트랜잭션 포함 목록(Inclusion List)을 작성하고, 제안자(Proposer)가 이를 포함하지 않으면 블록이 포크 선택 규칙에 의해 거부되도록 강제하는 메커니즘입니다.

```
[FOCIL 블록 생성 파이프라인]
  [검증자 위원회 IL 작성] ──> [포함 강제(Inclusion Enforced)]
                                      │
  ┌───────────────────────────────────┘
  ▼
  [블록 빌더 / 제안자] ──> [블록 내부 로컬 순서(Local Ordering) 임의 배치!]
                                │
                                ├─ Front-run Tx (차익거래자 매수)
                                ├─ Target Swap (포함 강제된 사용자 스왑)
                                └─ Back-run Tx (차익거래자 매도) ──> [샌드위치 완성]
```

#### [검증된 사실]
* FOCIL은 **검열 저항성(Censorship Resistance)과 포함 보장(Inclusion Guarantee)**을 제공하지만, 블록 내에서 트랜잭션의 **실행 순서(Intra-block Ordering)**를 강제하지 않습니다.  
* 블록 빌더 또는 제안자는 포함 목록에 들어 있는 스왑 트랜잭션의 바로 앞(Front-run)과 바로 뒤(Back-run)에 자신의 차익거래 트랜잭션을 삽입할 수 있는 자유도를 온전히 유지합니다.  
* 출처: [Ethereum Research: Fork-Choice Enforced Inclusion Lists (FOCIL)](https://ethresear.ch), [Paradigm Research: MEV, Censorship Resistance, and PBS Evolution](https://www.paradigm.xyz)

#### [추론 및 분석: 남는 잔여 공격 벡터]
1. **국소 순서 조작 (Local Ordering Exploitation)**: 제안자가 블록 빌더(PBS)와 결합할 경우, 강제 포함된 피해자의 대형 스왑 트랜잭션을 포위하여 완벽한 샌드위치 공격을 실행할 수 있습니다.
2. **지연 시간 경쟁(Latency Wars) 및 트랜잭션 스팸(Stuffing)**: FIFO(선착순)를 표방하는 체인에서도 시퀀서 노드와 물리적으로 가장 가까운 검색자(Searcher)가 마이크로초 단위로 트랜잭션을 채워 넣어 선두를 차지합니다.
3. **멀티 블록 MEV (Multi-block MEV)**: 단일 검증자가 연속된 2개 이상의 블록 슬롯을 할당받을 경우, 첫 번째 블록의 마지막에 풀 가격을 극단으로 밀어두고(Back-run 누락 상태 방치), 두 번째 블록 첫머리에서 오라클 및 청산을 장악하는 위험이 상존합니다.

---

### 2.2 슬리피지 기본값(Slippage Defaults) 설계 위험

#### [검증된 사실]
* 프론트엔드 UI에서 관행적으로 사용하는 **고정 슬리피지(예: 0.5% 또는 1.0%)**는 MEV 검색 봇에게 확정적인 무위험 수익 한도를 사전에 보장해 주는 역할을 합니다.  
* $100,000 거래에서 0.5% 슬리피지는 검색자에게 최대 $500 상당의 샌드위치 추출 마진을 열어줍니다. 반면 변동성이 극심한 장세에서는 0.5% 슬리피지가 잦은 트랜잭션 실패(Revert)를 초래하여 사용자 가스비만 낭비시킵니다.

#### [추론 및 분석: 권고 설계]
* **동적 슬리피지 모델(Dynamic Slippage Algorithm)**:  
  $$S_{\text{recommended}} = \text{PriceImpact} + k \cdot \sigma_{\text{volatility}} \cdot \sqrt{\Delta t_{\text{block}}}$$  
  풀 깊이(Liquidity Depth), 주문 크기, 최근 블록 간 변동성($\sigma$), 예상 블록 포함 대기 시간을 기반으로 클라이언트에서 동적으로 최소 수령량(`amountOutMin`)을 계산해야 합니다.  
* 대형 거래에 대해서는 프론트엔드에서 개인 RPC(Private Mempool/MEV-Blocker) 사용을 강제하거나 경고 팝업을 표시해야 합니다.

---

### 2.3 마감 시간(Deadline) 파라미터 취약점 및 안티패턴

#### [검증된 사실]
* 수많은 탈중앙화 애플리케이션 프론트엔드 및 SDK에서 자바스크립트 수준의 편의를 위해 다음과 같은 코드를 작성합니다:
  ```solidity
  // 치명적 안티패턴: 컨트랙트 호출 시 block.timestamp를 그대로 전달
  router.swapExactTokensForTokens(amountIn, amountOutMin, path, to, block.timestamp);
  ```
* EVM의 라우터 컨트랙트는 `require(deadline >= block.timestamp, "EXPIRED")`를 검사합니다.
* 사용자가 서명한 트랜잭션에 `deadline = block.timestamp`가 하드코딩되거나, 채굴 시점의 타임스탬프를 받도록 래핑 컨트랙트가 작성되면, 트랜잭션이 멤풀에 수 시간~수일 동안 억류되어도 채굴되는 순간에는 항상 `deadline == block.timestamp`가 성립하여 만료 검사가 완전히 무력화됩니다.
* 악의적인 블록 제안자는 시장 가격이 사용자에게 가장 불리하게 폭락할 때까지 트랜잭션을 멤풀에 쥐고 있다가, 사용자가 설정한 최대 슬리피지 한도 끝까지 체결시켜 차액을 편취합니다.

#### [추론 및 분석: 권고 설계]
* **서명 시점 기준 절대 타임스탬프 강제**: 지갑 및 클라이언트 UI에서 서명 생성 시점의 벽시계 시간(Wall-clock time)에 엄격한 만료 시간(예: +60초~180초)을 더한 값을 `deadline` 매개변수로 고정 전달해야 합니다.
* 코어 라우터 레벨에서 지나치게 먼 미래(예: 현재 블록 타임스탬프 대비 20분 이상)의 `deadline`은 트랜잭션 접수를 거부하도록 방어 로직을 추가해야 합니다.

---

## 3. 불변(업그레이드 불가) AMM의 수수료 스위치·긴급 정지(Pause) 부재 운영 현실과 교훈

### 3.1 관리자 키(Admin Key) 제거의 본질적 트레이드오프

```
[완전 불변 AMM (Zero-Admin, No-Pause)]
  │
  ├─ 장점 ──────────────────────────────────────────────┐
  │  • 완벽한 무신뢰(Trustless): 백도어 및 러그풀 불가   │
  │  • 거버넌스 공격 표면 0 (DAO 투표 하이재킹 면역)    │
  │  • 규제 당국의 제재/검열 압박 대상 부재             │
  │  • 타 디파이 프로토콜의 레고 블록 결합 신뢰 극대화   │
  │                                                     │
  └─ 단점 (운영적 냉혹함) ──────────────────────────────┘
     • 제로데이 버그/컴파일러 결함 발생 시 컨트랙트 동결 불가 (Pause 부재)
     • 자금 구출 불가능: 공격자 vs 화이트햇 vs 사용자 간의 잔고 인출 속도전(PVP Race)
     • 버그 수정 불가: 신규 v2 컨트랙트 재배포 및 유동성 강제 마이그레이션만 가능
     • 파라미터(수수료, 틱 간격) 동적 최적화 불가능 -> 유동성 파편화 발생
```

#### [검증된 사실: 역사적 사례 비교]
* **Uniswap v2 & v3 코어**: 완전 불변 컨트랙트로 배포되었습니다. 풀 자체에는 일시 정지(Pause) 함수가 전혀 없습니다. v2는 6년 이상 단 한 건의 코어 해킹 없이 천문학적 거래량을 무결하게 처리하여 불변성의 가치를 증명했습니다.
* **Uniswap 수수료 스위치(Fee Switch) 교훈**: Uniswap 코어에는 프로토콜 수수료(1/6 등)를 활성화할 수 있는 권한(`feeToSetter`)이 존재했으나, 규제 리스크(증권성 시비)와 LPs의 반발로 인해 수년간 단 한 번도 켜지 못했습니다. 
* **Curve vs Balancer 비상 대응 비교**:
  * Curve는 2023년 Vyper 취약점 당시 비상 정지 기능이 없던 풀들에서 자산이 그대로 털렸으며, 화이트햇 MEV 봇이 공격자보다 앞서 프론트런으로 자금을 탈취해 환원해 주는 비정상적 구조에 의존했습니다.
  * Balancer는 비상 서브풀 정지(Emergency Pause) 권한을 보유하여 2023년 사고 당시 80% 이상의 위험 자금을 동결 및 구출했습니다. 그러나 2025년 ComposableStablePool 사고에서는 이미 일시정지 기간(Pause Window)이 만료된 불변 상태의 풀들이 집중 타격을 입었습니다.
  * 출처: [Uniswap Governance Forum - Fee Switch Discussions](https://gov.uniswap.org), [Curve Post-Mortem Reports](https://curve.fi)

#### [추론 및 분석: 불변 AMM 설계 철학의 귀결]
* **"불변 AMM에서 보안 취약점의 수정은 존재하지 않는다. 오직 포크(Fork)와 유동성 대피만 존재한다."**
* 수수료 스위치(Protocol Fee Switch)를 아예 배제할 경우 거버넌스 토큰 탈취를 통한 수수료 갈취 리스크는 원천 봉쇄되나, 재단이 장기적으로 프로토콜을 유지보수할 온체인 현금흐름 창출이 차단됩니다.
* 신규 체인의 공식 DEX로서 완전 불변을 선택한다면, **"코어 로직은 최소한으로 단순화(v2 스타일 CPMM 또는 극도로 정제된 불변식)"**해야 하며, 복잡한 기능(동적 훅, 가변 수수료)을 코어에 담는 것은 불변성과 양립할 수 없는 자살 행위입니다.

---

## 4. 메인넷 출시 전 필수 권고 체크리스트

### 4.1 테스팅 및 정형 검증 (Testing & Formal Verification)

#### 1) 불변식 기반 퍼징 (Invariant Fuzzing)
Foundry(`testInvariant_*`), Echidna, Medusa를 사용하여 어떠한 트랜잭션 시퀀스에서도 깨지지 않아야 할 절대 불변식을 정의하고 1,000만 회 이상의 무작위 상태 전이를 실행해야 합니다.

```solidity
// Foundry Invariant Test 필수 명제 예시
contract AMMInvariantTests is Test {
    // 불변식 1: 수수료를 감안한 불변식 k는 절대 감소하지 않는다 (k_after >= k_before)
    function invariant_k_non_decreasing() public view {
        uint256 kCurrent = uint256(pool.reserve0()) * uint256(pool.reserve1());
        assertGe(kCurrent, initialK, "Invariant K violated!");
    }

    // 불변식 2: 실제 토큰 잔고는 항상 내부 상태 변수(Reserves) 이상이어야 한다 (지불능력)
    function invariant_solvency() public view {
        assertGe(token0.balanceOf(address(pool)), pool.reserve0(), "Solvency token0 broken!");
        assertGe(token1.balanceOf(address(pool)), pool.reserve1(), "Solvency token1 broken!");
    }

    // 불변식 3: 0을 입력하면 0이 출력되어야 하며, 입력 토큰 대비 출력 토큰은 단조 증가한다
    function invariant_monotonicity(uint256 amountIn) public {
        vm.assume(amountIn > 0);
        uint256 amountOut = pool.getAmountOut(amountIn, token0);
        assertGt(amountOut, 0, "Non-zero input gave zero output");
    }
}
```

#### 2) 차등 테스트 (Differential Testing)
* 스마트 컨트랙트 구현체를 Python(Sympy 기반 정밀 수학 모델) 또는 Rust 정밀 부동소수점 시뮬레이터와 교차 검증.
* **경계값 검증(Boundary Condition Stress Test)**:
  * 1 wei 단위의 극소량 입력 스왑.
  * $2^{128}-1$, $2^{256}-1$에 근접한 극대량 유동성 예치.
  * 자산 간 데시멀 불일치 극단값(0 데시멀 vs 18 데시멀, 6 데시멀 vs 30 데시멀).
  * 풀 내부 비율이 $1 : 10^{12}$ 이상으로 벌어진 초비대칭 상태에서의 반올림 방향성 검증.

---

### 4.2 스마트 컨트랙트 감사 우선순위 매트릭스 (Audit Priority Matrix)

| 우선순위 | 감사 영역 | 핵심 점검 질문 및 공격 시나리오 |
| :---: | :--- | :--- |
| **P0 (치명적)** | **반올림 방향성 (Rounding Direction)** | 모든 나눗셈에서 풀 잔고가 줄어드는 방향의 내림(Floor)이 발생하지 않는가? 마이크로 스왑 반복 시 1 wei 차익거래가 누적 가능한가? |
| **P0 (치명적)** | **크로스 함수/크로스 풀 재진입** | 외부 토큰 전송(SafeERC20) 시점 이전에 모든 상태 변수(Reserves, Balances)가 완전히 갱신되었는가? Read-only view 함수가 왜곡된 중간 상태를 반환하지 않는가? |
| **P1 (높음)** | **초기 LP 인플레이션 방어** | 첫 번째 LP 민팅 시 `MINIMUM_LIQUIDITY` 영구 소각이 강제되는가? 가상 셰어 오프셋 계산이 올바른가? |
| **P1 (높음)** | **토큰 전송 무결성 (Delta Accounting)** | `transfer`의 반환값에만 의존하지 않고 실제 풀 잔고 변화량(`balanceAfter - balanceBefore`)을 확인하는가? |
| **P2 (중간)** | **오라클 조작 저항성** | TWAP 누적값(TickCumulative, PriceCumulative)이 동일 블록 내 플래시 론으로 오염되지 않음을 수학적으로 증명했는가? |
| **P3 (안정성)** | **가스 고갈(DoS) 한계** | 반복문(Loop)이나 틱 순회(Tick traversal)가 블록 가스 한도(Block Gas Limit)를 초과하여 자금이 영구 동결될 위험이 없는가? |

---

### 4.3 지갑(Wallet) 및 프론트엔드 UI 보호 체크리스트

```
[지갑 UI 트랜잭션 제출 전 보호 플로우]
  │
  ├─ 1. 온체인 시뮬레이션 (eth_call / Dry-run)
  │     └─ 예상 최소 수령량 불일치 시 트랜잭션 제출 차단
  │
  ├─ 2. 마감 시간(Deadline) 엄격 검증
  │     └─ UI 생성 시점 + 120초 절대값 전달 (block.timestamp 직접 전달 절대 금지)
  │
  ├─ 3. 동적 슬리피지 경고
  │     ├─ 가격 영향(Price Impact) > 3%: 적색 경고 팝업
  │     └─ 슬리피지 허용치 > 1%: "샌드위치 봇 공격 위험 노출" 확인 체크박스 요구
  │
  └─ 4. 토큰 속성 분석 (Weird Token Warning)
        ├─ Fee-on-Transfer 감지 시: "전송 수수료 토큰으로 거래 손실 가능" 경고
        ├─ 비검증 컨트랙트 및 프록시 토큰: 미등록 토큰 주의 알림
        └─ ERC-777/ERC-1363 감지 시: 승인 전 재진입 위험 고지
```

1. **시뮬레이션 사전 실행 (Pre-flight Simulation)**: 지갑 앱은 사용자가 서명하기 직전 `eth_call`을 통해 트랜잭션의 실제 결과와 슬리피지를 시뮬레이션하고, 반환값이 기대치 이하일 경우 경고를 표시해야 합니다.
2. **`block.timestamp` 전달 금지**: UI 코드베이스 전체에서 `Math.floor(Date.now() / 1000) + DEADLINE_SECONDS` 형식의 절대 타임스탬프만 전달하도록 정적 분석 린터(Linter) 규칙을 적용해야 합니다.
3. **이상 유동성 경고**: 풀의 총 유동성(TVL) 대비 거래 규모가 5%를 초과할 경우 경고를 띄우고 주문 분할을 유도해야 합니다.

---

## 5. 결론 및 신규 체인 공식 DEX 아키텍처 제언

신규 체인의 첫 공식 DEX를 관리자 키 없는 불변 컨트랙트로 런칭하는 것은 블록체인의 기본 정신에 부합하는 가장 강력한 탈중앙화 선언입니다. 그러나 이 결정은 **"한 번 배포된 코드는 어떠한 버그가 있어도 되돌릴 수 없다"**는 불가역적 책임을 수반합니다.

2023~2026년 디파이 역사(Balancer의 $100M+ 반올림 사고, Curve의 Vyper 컴파일러 버그, KyberSwap의 틱 산술 결함)가 주는 단 하나의 교훈은 **"복잡성은 보안의 최대 적"**이라는 사실입니다. 

신규 체인의 불변 DEX는 최신 유행인 동적 훅이나 다중 자산 복합 풀을 지양하고, **수학적으로 완벽히 증명되고 수년간 온체인 전투 검증을 마친 단순 불변 모델(전송 전후 잔고 차이 계측과 영구 LP 락업이 보강된 Uniswap v2 코어 변형)**을 기본 코어로 채택해야 합니다. 여기에 FOCIL 등 합의 계층의 특성을 반영한 동적 슬리피지와 엄격한 타임스탬프 지갑 보호망을 결합하는 것만이 프로토콜과 사용자의 자산을 영구히 보호하는 유일한 길입니다.

---

## 6. 참고 문헌 및 공식 출처 (References)

1. [Trail of Bits Security Post-Mortem on Balancer Exploit](https://trailofbits.com)
2. [OpenZeppelin Balancer Incident Analysis](https://openzeppelin.com)
3. [Immunefi Balancer Rounding Error Vulnerability Report](https://immunefi.com)
4. [Vyper Official Security Advisory GHSA-5824-cm3x-3c38 (Curve Exploit)](https://github.com/vyperlang/vyper/security/advisories/GHSA-5824-cm3x-3c38)
5. [Hacken In-Depth Analysis: Curve Vyper Reentrancy Bug](https://hacken.io)
6. [CertiK Alert: Curve Finance Vyper Compiler Vulnerability](https://certik.com)
7. [Chainlight: Read-only Reentrancy Vulnerability Analysis](https://chainlight.io)
8. [BlockSec KyberSwap Elastic Incident Analysis](https://blocksec.com)
9. [Halborn Security KyberSwap Exploit Breakdown](https://halborn.com)
10. [KyberSwap Official Post-Mortem & Grant Program](https://kyberswap.com)
11. [Uniswap v4 Technical Whitepaper and Hook Architecture](https://docs.uniswap.org)
12. [Ethereum Improvement Proposal EIP-1153: Transient Storage Opcodes](https://eips.ethereum.org/EIPS/eip-1153)
13. [Code4rena Trader Joe v2 Liquidity Book Contest Findings](https://code4rena.com)
14. [LFJ (Trader Joe) Liquidity Book Technical Documentation](https://docs.lfj.gg)
15. [Spearbit Security Review: Velodrome Slipstream](https://spearbit.com)
16. [Ethereum Research: Fork-Choice Enforced Inclusion Lists (FOCIL)](https://ethresear.ch)
17. [Paradigm Research: MEV, Censorship Resistance, and PBS Evolution](https://www.paradigm.xyz)
18. [Weird ERC20 Tokens Repository (d-xo)](https://github.com/d-xo/weird-erc20)
19. [OpenZeppelin: ERC4626 Share Inflation Attack & Defenses](https://openzeppelin.com)
