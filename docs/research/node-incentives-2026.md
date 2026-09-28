# 노드 보상 설계 비교 조사 (2026-09) — Aether (A)(B)(C) 평가

조사일 2026-09-28. 1차 출처(스펙·공식 문서·EIP/SIMD/HIP·재단 블로그) 위주로 확인했고, 2차 출처로만 확인한 수치에는 "(2차)"를 붙였다.
평가 대상:
- (A) 시간 단위 노드 보상 + 증명자 보상, 운영자별 1/16 상한
- (B) 연속 가동(로열티) 가중치 1x→2x
- (C) 오래된 이력을 저장하면 받는 1.5x 가중치

---

## 1. 가동률·로열티 보상은 다른 체인에서 어떻게 하나

### 1.1 Ethereum: 매 에폭 출석 보상과 비활성 누수
- 에폭(6.4분)마다 attestation을 보내는데, 보상은 source·target·head 투표가 "제때" 들어갔는지로 정해진다. 가중치는 64 중 14/26/14이고, sync committee가 2, 제안자가 8이다([consensus-specs], [ethereum.org rewards]).
- 늦거나 빠진 투표는 받을 수 있던 보상만큼 감점된다. 페널티가 보상과 같은 크기라서 **대칭**이다. 가동률 누적 보너스는 없고, 매 에폭 새로 셈한다.
- 비활성 누수(inactivity leak): finality가 4 에폭 넘게 멈출 때만 발동한다. 비활성 점수는 결석한 에폭마다 +4, 출석한 에폭마다 −1이다(`INACTIVITY_SCORE_BIAS=4`, `INACTIVITY_PENALTY_QUOTIENT=2^24`). 계속 꺼져 있으면 잔고가 대략 exp(−N²/2Q)로 줄어든다([eth2book inactivity]).
- **시사점:** 결석에는 벌을 주지만 "오래 있었다"에는 보너스를 주지 않는다. 점수가 결석 시 빨리 오르고 출석 시 천천히 내려가는 비대칭 감쇠가 (B)의 "하루에 하루씩 감소"와 개념이 같다.

### 1.2 Avalanche: 이진 가동률 문턱(80%에서 90%로)
- 스테이킹 기간 전체 가동률이 문턱 미만이면 검증자와 위임자의 보상이 **0**이다. ACP-267로 문턱이 80%에서 90%로 올랐고, 2026-04-01 이후 시작한 검증부터 적용된다([ACP-267], [Avalanche blog ACP-267]).
- 가동률은 다른 검증자들이 관측해 매기는 주관적 값이다. 문턱 근처에서 전부 아니면 전무가 되어 절벽 효과가 크다.
- **시사점:** 이진 문턱은 설명이 쉽다. 대신 89%와 90%의 차이가 보상 100%라서 억울한 사례가 나온다. Aether는 이미 시간 단위라서 절벽이 작다.

### 1.3 Cosmos SDK: 다운타임 jail
- `signed_blocks_window` 동안 `min_signed_per_window`(보통 5~50%) 미만으로 서명하면 jail된다. 다운타임 슬래시는 보통 0.01%이고, Cosmos Hub의 `downtime_jail_duration`은 10분이다([Cosmos SDK x/slashing], [Secret docs], [Cosmos forum #11783]).
- Hub 포럼에서는 다운타임 슬래시를 없애고 jail만 남기자는 제안이 논의됐다(2023). 다운타임은 악의가 아니라서 벌금보다 "보상 없음 + 퇴출"이 맞다는 논리다.
- **시사점:** 선의의 다운타임에는 슬래시 말고 보상 미지급이 표준으로 굳어지고 있다. Aether의 "remainder never issued"와 같은 방향이다.

### 1.4 Solana: vote credits, TVC, SFDP
- SIMD-0033 Timely Vote Credits는 2024-11(epoch 703)에 활성화됐다. 투표 지연이 2슬롯 이하이면 16 크레딧이고, 이후 한 슬롯마다 1씩 줄어 최소 1이다([SIMD-0033], [Figment TVC]).
- 연속량으로 보상하므로 이진이 아니다. "느리게라도 참여"와 "빠른 참여"를 구분한다.
- SFDP(재단 위임): 첫해 투표비를 100%에서 75%, 50%, 25%로 분기마다 줄여 준다. 성과 기준 미달 에폭에는 지원이 0이다. 2025-10에는 매칭을 줄여 재단 의존을 낮췄다([SFDP updates], [SFDP criteria]).
- **시사점:** 신규 참여자 지원을 기간 한정으로 점점 줄이는 방식은 효과가 있었다. 다만 재단 재량이라 중앙화 비판을 받았다. Aether는 재단 재량을 없앤 대신, 프로토콜 규칙으로 할 수 있는 것만 남는다.

### 1.5 Sui / Aptos: 성과 기반 보상
- Sui: 검증자끼리 서로 점수를 매기는 tallying rule이 있고, 신고된 검증자는 그 에폭 보상이 삭감된다([Sui validator rewards]).
- Aptos: 보상 = 스테이크 × 보상률 × 제안 성공률(proposal success rate)([Aptos leaderboard], [Staking Rewards Aptos]).
- Aptos 검증자 수는 2024-10의 146명에서 2026-09의 84명으로 약 42% 줄었다(2차). 보상률을 낮추자 영세 운영자부터 빠졌다.
- **시사점:** 성과에 비례해 곱하는 방식은 자연스럽다. 다만 보상 총액이 줄면 소규모 운영자가 먼저 이탈한다.

### 1.6 Internet Computer: 노드 제공자 성과 기반 보상(PBR)
- 월간 보상에 곱수를 적용한다. 서브넷 75퍼센타일 대비 상대 실패율이 10% 이하면 1.0이고, 10~60% 구간에서는 곱수가 선형으로 줄어 60%에서 0.2가 된다. **하한은 0.2**다([ICP PBR wiki], [ICP remuneration]).
- 절대 기준이 아니라 **서브넷 동료 대비 상대 기준**이다. 네트워크 전체 장애 때 모두가 벌받지 않는다.
- **시사점:** "남들도 다 끊겼으면 봐준다"는 상대 기준은 Mac 가정용 네트워크(ISP 장애, macOS 업데이트)에 잘 맞는다.

### 1.7 Helium: Proof of Coverage와 게이밍
- 비콘·위트니스로 커버리지를 증명한다. 거짓 위치(스푸핑)가 만연해 2022년 denylist를 투표로 도입했다. 52.4만 핫스팟 중 약 2.5만 개(약 5%)가 등재됐다(2차, [Helium denylist HIP-40], [HIP-66 trust score]).
- 반감기: HNT는 2년마다 반감하며, 2025-08-01에 연 1,500만에서 750만 HNT가 됐다([Helium halving 2025]). 데이터 전송 보상은 반감과 무관하다.
- 데이터 전용 핫스팟은 PoC 보상 없이 온보딩비 $0.50로 무허가 가입한다(HIP-123). 보상이 없으니 sybil 동기도 없다.
- **시사점:** 하드웨어와 위치를 인증해도 **증명 대상이 "주장"이면 스푸핑은 끝나지 않는다.** 보상은 검증 가능한 것에만 붙여야 한다.

### 1.8 POKT / Theta
- POKT Shannon(2025-06 전환): 실제 릴레이 수에 비례해 보상한다(Relay Mining, 확률적 증명). QoS 실패 릴레이는 제외된다([POKT Shannon]).
- Theta Edge Node: 가동률 비례 "uptime mining"과 TFUEL 스테이크(10k~500k)를 결합한다([Theta Edge FAQ], [Theta EEN staking]).
- **시사점:** "켜져 있음"만 보상하는 설계는 결국 "일한 양"을 함께 보는 쪽으로 이동했다.

### 1.9 명시적 연공 가중치(coin-age, 스테이크 숙성)
- Peercoin: coin age는 30일이 지나야 쓸 수 있고, 90일에 상한이 걸린다. 오래 잠든 부자가 한꺼번에 블록을 독점하는 것을 막으려는 상한이다. 그래도 coin age는 **평소 온라인 유인을 약화**시켰고, 51% 공격 문턱도 낮췄다. 이후 PoS 설계는 대체로 coin age를 피한다([Peercoin docs], [Pulsar arXiv 2411.14245]).
- Filecoin: 섹터가 오래됐다고 보너스를 주지 않는다. 대신 조기 종료 페널티와 담보(pledge)로 장기 약정을 유도한다(아래 3.5).
- **교훈:** "오래됐다"는 이유만으로 주는 보너스는 ① 기득권 고착 ② 신규 진입 억제 ③ 휴면 후 복귀 공격 경로를 만든다. 안전한 형태는 상한이 짧고, 끊기면 감쇠하며, "보너스"가 아니라 "워밍업"으로 설명되는 것이다.

---

## 2. 초기 참여자와 부트스트랩

| 사례 | 메커니즘 | 결과·교훈 |
|---|---|---|
| Filecoin baseline minting | 채굴 할당의 30%는 단순 감쇠로, 70%는 네트워크 저장량이 baseline 목표를 넘을 때만 발행([Filecoin spec minting]) | 초기 과잉 보상을 막았다. 반대로 성장이 목표에 못 미치면 발행이 지연되고, 모델이 복잡해 이해도가 낮았다 |
| Helium | 2년 반감, 초기 핫스팟 보상 큼 | 초기 수익이 "투자 상품"처럼 팔렸다(제3자 호스트 판매). 스푸핑이 폭증했다 |
| Bitcoin | 공정 출시, 초기 CPU 채굴 | 초기 한 채굴자(Patoshi)가 약 100만 BTC를 보유한 것으로 추정된다. 공정해도 초기 집중은 생긴다 |
| Kaspa | 2021-11 공정 출시, 프리마인 없음. 첫 반년 이후 연 반감을 월 (1/2)^(1/12)로 매끄럽게 적용([Kaspa tokenomics]) | 월 단위 매끄러운 감쇠라서 반감기 절벽 이벤트가 없다 |
| Grin | 블록당 60 grin 영구 선형 발행 | 초기 우대가 없다. 대신 초기 가격 붕괴와 개발 자금 부족을 겪었다 |
| Chia | 2,100만 XCH 프리팜(전략 준비금) | 신뢰 논란이 이어졌다. Aether의 "프리마인 없음"이 차별점이다 |
| 인센티브 테스트넷(Aptos, Celestia 등) | 테스트넷 활동을 에어드랍 | 봇·다지갑 파밍이 만연했다. 사후 sybil 필터링은 사람 손이 많이 가고 논란이 컸다 |

**교훈 요약.**
1. 명시적 "얼리버드 배수"는 거의 모두 투기·파밍·규제 문제를 불렀다.
2. 초기 이점은 "경쟁자가 적어서 생기는 것"으로 두는 편이 법적·도덕적으로 가장 깔끔하다. Aether의 현재 방침이 이것이다.
3. N<16이면 미발행분을 소각이 아니라 **처음부터 발행하지 않는다.** 이 구조가 Filecoin baseline의 "성장 전 과잉 발행 금지"를 훨씬 단순하게 구현한다.

---

## 3. 저장·이력 보상

### 3.1 Ethereum: EIP-4444, Portal, era 파일
- 2025-07-08부터 모든 EL 클라이언트가 머지 이전 이력을 잘라낼 수 있다(partial history expiry). 절감량은 300~500 GB다([EF blog 2025-07-08]).
- 이력은 era1/era 파일(8192 블록 단위)로 배포되고, Portal 네트워크와 archive 제공자가 보관한다. **프로토콜 차원의 보상은 없다.** 선의와 기관(EF, 클라이언트팀, 인프라 업체)에 기댄다([eth-clients history-endpoints], [Portal specs]).
- **시사점:** era 파일 형식(8192 블록)은 Aether (C)의 단위와 같아서 도구를 재사용할 수 있다. 무보상으로도 "누군가는 보관"은 되지만, 소수 기관에 집중된다.

### 3.2 Solana: Old Faithful, BigTable
- 이력은 Google BigTable 인스턴스 5~6개에 의존해 왔다. Old Faithful(Triton, 재단 지원)이 전체 원장(약 250 TB 이상)을 CAR 파일로 Filecoin·S3에 올렸다([Old Faithful docs], [Triton report]).
- 온체인 인센티브는 없고 재단 보조금에 기댄다. 체인이 커지면 개인이 보관할 수 없는 규모가 된다.

### 3.3 Celestia / Avail / EigenDA
- Celestia: DAS 샘플링 창은 7일이고(CIP-36), 최소 프루닝 창은 30일에서 7일+1시간으로 줄었다(CIP-34). 아카이브는 별도 운영자가 유료 제공한다([Celestia Matcha blog]).
- EigenDA: 청크를 저장하지 않고 서명하는 것을 막으려고 proof-of-custody를 쓴다. 저장한 청크 전체로만 계산 가능한 값을 제출하게 한다. 슬래시는 아직 미가동이다(2차).
- Avail: KZG 커밋과 라이트클라이언트 샘플링을 쓴다. 저장 보상은 검증자 보상에 포함된다.
- **시사점:** DA 계층도 장기 이력은 인센티브 밖에 둔다. 단기 창만 합의가 보장한다.

### 3.4 Walrus(Red Stuff)
- 2차원 소거 부호로 복제 계수는 약 4.5x다(완전 복제는 25x, 1D RS는 3x지만 복구 비용이 크다). 에폭 말에 무작위 코인(2f+1 임계)으로 도전 대상 blob을 정한다. 노드는 슬리버를 서로 교환해 2f+1 서명으로 Certificate of Storage를 온체인에 제출한다([Walrus paper arXiv 2505.05370]).
- **비동기 도전**이라서 네트워크 지연을 악용해 남에게서 심볼을 모아 답하는 공격이 구조적으로 막힌다(정직 노드 f+1이 응답하지 않음). 슬래시는 아직 미가동이고, 보상은 dPoS 스테이크 비례다([Walrus PoA blog]).
- **시사점:** 무거운 설계(DKG, 전원 교환)다. 단일 Mac 운영자 수천 명에게는 과하지만, 도전 시각을 사전에 알 수 없게 하는 원리는 가져올 만하다.

### 3.5 Filecoin: PoRep과 WindowPoSt
- PoRep(SDR 봉인): 복제본마다 고유하게 인코딩한다. 봉인이 수 시간 걸리는 것을 이용해 **sybil(복제 가장), 외주(outsourcing), 생성(generation) 공격**을 막는다([PoRep report], [PL research]).
- WindowPoSt: 24시간을 30분 마감 48개로 나누고, 모든 섹터를 하루 1회 증명한다. 누락 시 섹터 장애 수수료는 하루 기대 보상보다 조금 크다([Filecoin spec PoSt], [Filecoin docs proving]).
- 비용: 봉인에 GPU와 대용량 RAM이 필요하다. **평범한 Mac에서는 비현실적**이다.

### 3.6 Arweave: SPoRA, replica.2.9 packing
- 채굴 자격은 무작위 청크에 접근할 수 있음을 증명해야 얻는다. 청크는 채굴자 주소로 키잉된 RandomX 기반 packing으로 저장한다. "필요할 때 packing"하는 것보다 "packing해서 디스크에 두는 것"이 싸도록 비용을 설계했다. 2.9에서 packing이 가벼워졌다([Arweave syncing-packing], [SPoRA post]).
- **시사점:** 공개 데이터라서 생기는 외주 문제를 운영자별 고유 인코딩 + 비대칭 비용으로 푸는 대표 사례다. Aether (C)가 가장 참고할 대상이다.

### 3.7 Storj / Sia / Swarm: 저렴한 무작위 감사
- Storj: 위성이 무작위 조각을 감사한다. 감사 점수가 96% 미만이면 영구 실격이고, 100% 실패 시 40회 만에 실격된다([Storj forum new audit scoring], [Storj docs DQ]).
- Sia: 계약 기간 끝에 이전 블록 ID로 정한 무작위 세그먼트의 Merkle 증명을 낸다. 실패하면 호스트 담보를 몰수한다([Sia whitepaper], [Sia docs]).
- Swarm: 이웃(neighbourhood) 단위 reserve 샘플링을 commit-reveal로 한다. 일치한 노드 중 스테이크 비례 추첨으로 보상한다([Swarm mechanics], [Redistribution.sol]).
- **공통점:** 검증은 Merkle 경로 하나로 끝나 매우 싸다. 외주 방어는 약하다. 대신 **담보, 실격, 저장 자체에 대한 사용자 지불**로 보완한다.

### 3.8 가벼운 도전과 무거운 복제증명 비교

| 부류 | 예 | 검증 비용 | 외주·생성 방어 | Mac 적합성 |
|---|---|---|---|---|
| 무작위 Merkle/bao 도전 | Sia, Storj, Swarm, Aether(C) 초안 | μs~ms | 약함. 공개 데이터면 즉석 fetch로 통과 | 매우 좋음 |
| 운영자 키잉 packing + 짧은 마감 | Arweave | ms(검증), 수 초(packing) | 중간~강함 | 좋음(경량 packing이면) |
| 봉인 PoRep + PoSt | Filecoin | SNARK 검증 | 강함 | 나쁨 |
| 비동기 교차 도전 | Walrus | 온체인 인증서 | 강함 | 운영 복잡 |

---

## 4. Sybil과 게이밍

- **다지갑:** 운영자 = 지갑이면 한 사람이 지갑을 여러 개 만든다. 한계는 "기기 1대 = 등록 1회"뿐이다. 즉 **1/16 운영자 상한은 사람 단위 상한이 아니다.** 실제 상한은 Mac 대수(구입비)다. Helium도 "핫스팟 1대 = 1 보상 단위"여서 대량 구매자가 생겼다.
- **켜기/끄기 게이밍:** 비콘 순간에만 켜 두는 행위다. Ethereum과 Solana는 매 슬롯·에폭 투표라서 불가능하고, Helium은 6시간 비콘을 약 12개 위트니스가 관측한다(2차).
- **외주 증명:** Filecoin은 PoRep 시간 가정, Arweave는 packing, Walrus는 비동기 도전으로 막는다. 공개 이력 데이터를 무작위 청크로 도전하면 **외주가 기본적으로 가능**하다.
- **하드웨어 인증:**
  - Helium: 제조사 보안칩(ECC) 키, 제조사 승인 온보딩.
  - Worldcoin: Orb 홍채.
  - IoTeX: 디바이스 신원(ioID/DID).
  - Apple DeviceCheck: 앱·기기마다 2비트 상태를 Apple 서버에 저장한다. 재설치해도 남는다. 검증에는 개발자 JWT로 Apple 서버를 호출해야 한다([Apple WWDC21 fraud], [Approov limits]).
  - **App Attest는 macOS 27부터 지원한다**(WWDC26). Secure Enclave 키가 진짜 Apple 기기의 무결성(Full Security, SIP)과 함께 있음을 증명한다. 재설치나 복원하면 키가 무효가 된다([WWDC26 App Attest]).
  - **Aether에 중요한 함의:** DeviceCheck만 쓰면 등록 순간에만 "진짜 Mac"이 증명된다. 이후 비콘 서명 키가 Secure Enclave 안에 있다는 증명은 없다. 한 번 등록한 뒤 소프트웨어 키로 서버·VM에서 비콘을 보내는 경로가 열려 있다.

---

## 5. Aether 평가

평가 축: 검증 비용 / Mac 1대 실현성 / 설명 쉬움 / sybil·게이밍·외주 저항 / 중앙화·기득권 / 법적 위험.

### (A) 시간 단위 노드 보상 + 증명자 보상, 1/16 상한 — **변경 후 채택**

잘한 점:
- 매시간 새로 세서 이탈에 강하다. Ethereum이 매 에폭 새로 세는 것과 같은 계열이다.
- 미발행 잔여분이 Filecoin baseline의 목적을 단순하게 달성한다.
- 이익 약속이 없는 작업 대가라서 SEC의 2025 PoW 채굴·프로토콜 스테이킹 staff 견해("관리적·사무적 활동 ≠ 증권")와 결이 같다([SEC PoW 2025-03-20], [SEC staking 2025-05-29]).

문제:
1. 비콘 1회/시간은 켜기/끄기 게이밍에 뚫린다. 이미 알고 있는 문제다.
2. 1/16은 지갑 단위라서 다지갑 앞에서는 장식이다. N≥16이면 사실상 "Mac당 1/N"이다.
3. 등록 후 비콘 키가 기기에 묶여 있지 않다(4절).

구체 변경:
- **비콘 슬롯:** 시간당 4 슬롯을 둔다. 각 슬롯 시각은 직전 시간 마지막 블록 해시로 정하므로 미리 알 수 없다. 슬롯 공지 후 **90초 안에** Secure Enclave 키로 (슬롯 난수 ‖ 최근 블록 해시)에 서명해 제출한다.
- **연속 배분:** 그 시간 몫을 (응답 슬롯 수/4)로 곱한다. 이진 대신 Solana TVC식 연속량으로 절벽을 없앤다. 0/4면 0이다.
- **기기 재증명:**
  - 매일 무작위 1개 비콘에 새 DeviceCheck 토큰을 첨부한다. 토큰은 실제 기기에서만 생성된다.
  - macOS 27 이상에서는 등록 시 App Attest assertion을 필수로 하고, 이후 비콘 서명을 App Attest 키로 한다.
  - Apple API 검증자는 오프체인이다. 그러니 검증 결과(기기 ID 해시, 날짜)만 온체인에 커밋하고, 누구나 재검증할 수 있도록 원본 토큰 해시를 공개한다.
- **문서 표현:** "1/16은 소수일 때 과잉 발행을 막는 장치이지, 한 사람의 몫을 제한하지 않는다"고 명시한다. 과장된 탈중앙 주장은 법적으로도 위험하다.
- **증명자:** 등록 운영자 + 1/16 상한은 유지한다. 증명 1건 = 검증 가능한 작업이므로 게이밍 여지가 작다.
- **비용:** 슬롯당 서명 1개(P-256) 검증, 시간당 4N개. N=10,000이면 약 11 tx/s다. 시스템 트랜잭션(수수료 0, 크기 약 100 B)으로 처리하거나, 블록 제안자가 비콘을 모아 Merkle 루트와 비트맵으로 기록한다.

### (B) 로열티 가중치 1x→2x — **변경 후 채택("워밍업"으로 재정의)**

잘한 점:
- "하루에 하루씩 감쇠, 리셋 없음"은 Ethereum 비활성 점수처럼 관대하면서 호핑을 막는다.
- 상한이 30일로 짧아 Peercoin식 무한 누적이 없다.

문제:
1. 95% 일일 가동률은 노트북 Mac(잠자기, 이동)에 가혹하다. Avalanche도 90%이고 기간 전체 기준이다.
2. "2배"라는 단어는 고참 우대(기득권)나 얼리버드 수익률처럼 읽힌다. 법적 표현 위험이 있다.
3. 2x는 신규 운영자가 첫 달에 절반만 받는다는 뜻이라 진입 장벽이 된다. 곡선 자체는 괜찮다.

구체 변경:
- **표현 반전:** 신규 Mac은 "워밍업 0.5"에서 시작해 "정상 1.0"에 도달한다. 수학은 base 1/32 × weight(1~2)와 **동일**하다. 상한은 weight 1.0에서 1/16이다. "보너스"라는 단어를 쓰지 않는다.
- **기간 단축:** 30일 대신 **14일**. 하루 +1/14씩 오른다. 한 달 기다림은 Mac 사용자 이탈을 키운다.
- **"좋은 날" 기준:** 95% 대신 **그날 비콘 슬롯 96개 중 86개 이상(약 90%) 응답**. 비콘 슬롯 기준으로 세므로 (A)와 같은 데이터를 재사용하고 검증 비용이 0이다.
- **감쇠:** 나쁜 날에는 −1/14, 즉 하루에 하루씩으로 유지한다. 단 **네트워크 전체 응답률이 70% 미만인 날은 중립**(오르지도 내리지도 않음)으로 둔다. ICP의 상대 기준을 차용해 대규모 장애나 macOS 업데이트 날을 보호한다.
- **상한 유지:** 워밍업 끝(1.0)이 최대다. 장기 근속 추가 보너스는 영구 금지한다. Peercoin 교훈이다.
- **감사:** 가중치는 운영자가 아니라 Mac(등록 ID) 단위로 저장한다. 지갑을 바꿔도 이전되지 않게 해서 워밍업 된 Mac 매매 시장을 억제한다.

### (C) 저장 가중치 1.5x — **변경 후 채택(단계적 도입, 초기 배수 축소)**

잘한 점:
- era 파일(8192 블록)은 Ethereum 도구와 호환된다.
- RS 16-of-32는 저장 오버헤드 2x로, 샤드 절반이 사라져도 복구된다.
- bao/BLAKE3 경로 검증은 μs 수준이고, 온체인 era 루트로 무신뢰 검증이 된다. Sia·Storj류의 가장 싼 검증이다.

문제:
1. **외주 공격이 기본적으로 통과한다.** 이력은 공개 데이터라서 도전받은 청크(1 KiB)를 다른 Aether 노드나 CDN에서 몇 ms 만에 가져와 bao 경로를 만들 수 있다. 시간당 1회 도전이면 저장하지 않고도 1.5x를 받는다. Filecoin과 Arweave가 PoRep/packing을 만든 이유가 정확히 이것이다.
2. 샤드가 결정적(RS 공개 부호)이라서 누구나 같은 샤드를 계산할 수 있다. 공개 샤드를 한 곳에서 서빙하면 모두가 통과한다. 결국 모두가 한 서버에 기대는 중앙화로 수렴한다.
3. 1.5x는 크다. 게이밍 수익이 커서 공격 동기가 강하다.

구체 변경:
- **1단계(메인넷 초기): 가중치 1.0 고정, 보상 없는 "의무 + 가용성 측정"만.** 도전은 받되 성공률만 공개 통계로 쌓는다. 외주 비율을 관측할 데이터를 먼저 확보한다.
- **2단계: 운영자 키잉 경량 packing.**
  - 청크 c에 대해 저장값 = c ⊕ H_packed(era_root, shard_idx, chunk_idx, **device_id**)로 둔다.
  - H_packed는 청크당 Mac에서 약 50~200 ms 걸리는 순차 메모리 하드 함수다(예: BLAKE3 기반 반복, 또는 Arweave replica.2.9식 엔트로피 사전 생성).
  - 운영자는 packed 샤드의 bao 루트를 할당 시 온체인에 커밋한다.
  - 검증자는 무작위로 k개 청크를 unpack해 원본 era 루트와 대조한다. 스팟 체크 1회로 커밋 정직성을 확률적으로 보장한다.
  - **응답 마감을 packing 비용보다 짧게(예: 2초)** 두고 한 번에 16개 청크를 도전한다. 즉석 생성에 16×100 ms 이상 걸려 마감을 넘기게 해서, 저장이 즉석 생성보다 싸도록 만든다.
- **도전 빈도:** (A)의 비콘 슬롯마다 1회, 즉 시간당 4회. 각 16 청크. 검증은 bao 경로 16개로 수 ms다.
- **배수:** 2단계 시작은 **1.2x**. 외주율이 5% 미만으로 6개월 관측되면 1.5x로 올리는 것을 거버넌스 안건으로 둔다. 상한 1/16은 유지한다.
- **실패 처리:** 도전 실패 시 그 시간 저장 배수 0. 7일 중 3일 이상 실패하면 샤드 할당을 해제하고 14일 재할당을 금지한다. 슬래시는 없다(Cosmos 교훈, 선의 장애 보호).
- **디스크 상한:** 운영자당 기본 할당 50 GB, 선택 200 GB. Mac 디스크 여유를 해치지 않도록 앱에서 명시적으로 동의받는다.

---

## 6. 비교표

| 체인 | 메커니즘 | 파라미터 | Aether 판정 |
|---|---|---|---|
| Ethereum | 에폭 단위 적시 투표 보상, 비활성 누수 | 가중치 14/26/14/2/8 (/64), 결석 +4 / 출석 −1 | 매 시간 새로 세기와 비대칭 감쇠 **채택** |
| Avalanche | 이진 가동률 문턱 | 80%→90%(ACP-267, 2026-04) | 문턱 90%는 **참고**, 이진은 **불채택**(연속량 사용) |
| Cosmos SDK | 다운타임 jail, 소액 슬래시 | window, min 5~50%, 0.01%, jail 10분 | 슬래시 없이 미지급만 **채택** |
| Solana | TVC, SFDP | 지연 ≤2슬롯 16크레딧, 투표비 100→25% 감소 | 연속 크레딧 **채택**, 재단 재량 지원 **불채택** |
| Sui / Aptos | 동료 평가, 제안 성공률 곱 | tallying, stake×rate×success | 곱셈형 **채택**, 동료 평가 **불채택**(담합 위험) |
| ICP | 상대 실패율 곱수 | 10%까지 1.0, 60%에서 0.2, 하한 0.2 | 전체 장애일 중립 처리 **채택** |
| Helium | PoC 비콘·위트니스, 반감 | 2년 반감, 2025-08 연 750만 HNT, denylist 약 5% | "주장 대신 검증 가능한 것만 보상" 교훈 **채택** |
| POKT / Theta | 릴레이 비례, 가동률+스테이크 | Relay Mining, TFUEL 10k~500k | 가동률 단독 보상 한계 **참고** |
| Peercoin | coin age | 30일 최소, 90일 상한 | 장기 누적 보너스 **불채택**, 짧은 상한만 |
| Filecoin | baseline minting, PoRep/WindowPoSt | 30/70 분할, 30분 마감 ×48, 장애 수수료 > 하루 보상 | 미발행 잔여 방식 **유지**, PoRep **불채택**(Mac 불가) |
| Arweave | SPoRA + 키잉 packing | RandomX, replica.2.9 | 경량 packing **채택**(C 2단계) |
| Walrus | Red Stuff 2D, 비동기 도전 | 약 4.5x, 2f+1 인증 | 예측 불가 도전 시각만 **채택** |
| Storj / Sia / Swarm | 무작위 Merkle 감사, 담보, 추첨 | 감사 96% 미만 실격, 40회, 계약 담보 | bao 도전 **채택**, 실격 규칙 **완화 채택** |
| Ethereum 이력 | EIP-4444, era, Portal(무보상) | 8192 블록 era, 2025-07-08 | era 형식 **채택**, 무보상 한계를 (C)로 보완 |
| Solana 이력 | BigTable, Old Faithful | 약 250 TB 이상, 재단 보조금 | 재단 의존 **불채택** |
| Kaspa / Grin / Chia | 공정 출시, 선형, 프리팜 | 월 (1/2)^(1/12), 60/블록, 2,100만 | 프리마인 없음 **유지**, 얼리버드 배수 **불채택** |
| Apple 인증 | DeviceCheck 2비트, App Attest | macOS 27부터 App Attest | 일일 DeviceCheck 재증명 + 27 이상에서 App Attest **채택** |

---

## 7. 법적 표현 점검 (얼리버드·수익 약속 금지)

- **금지 표현:** "초기 참여 시 2배", "연 X% 수익", "지금 시작하면 더 많이", "Mac 투자 회수 기간". Helium 핫스팟이 "수익 기계"로 판매된 것이 반면교사다.
- **허용 표현:**
  - "이 시간에 네트워크 일을 한 Mac 수로 나눈다."
  - "새 Mac은 2주 워밍업이 있다."
  - "보상량은 참여자 수와 발행 일정에 따라 변하며 보장되지 않는다."
- **근거:** 미국 SEC staff는 PoW 채굴(2025-03-20)과 프로토콜 스테이킹(2025-05-29)을 "관리적·사무적 활동"이라서 증권 거래가 아니라고 봤다. 핵심은 **타인의 경영 노력에 기대는 수익 기대**가 없어야 한다는 점이다. staff 견해일 뿐 법적 구속력은 없다. 한국 가상자산이용자보호법 등 관할별 검토는 별도로 필요하다.
- (B)의 "2x", (C)의 "1.5x"는 UI에서 배수로 보여 주지 말고 "정상 몫 대비 %"로 표시한다.

---

## 8. 결론 요약

| 규칙 | 판정 | 핵심 변경 |
|---|---|---|
| (A) 시간 보상 + 1/16 | **변경 후 채택** | 시간당 예측 불가 슬롯 4개와 90초 응답, 슬롯 비율 연속 배분, 일일 DeviceCheck 재증명, macOS 27 이상 App Attest, "1/16은 사람 상한 아님" 명시 |
| (B) 로열티 1x→2x | **변경 후 채택** | "워밍업 0.5→1.0"으로 재표현, 14일, 좋은 날 = 슬롯 90% 이상, 감쇠 하루씩, 전체 장애일 중립, Mac 단위 저장 |
| (C) 저장 1.5x | **변경 후 채택(단계적)** | 1단계 무보상 측정, 2단계 기기 키잉 경량 packing + 2초 마감 + 16청크 도전 ×4/시간, 초기 1.2x, 실패 시 슬래시 없이 할당 해제 |

---

## 출처

- [consensus-specs] https://github.com/ethereum/consensus-specs (altair beacon-chain: weights)
- [ethereum.org rewards] https://ethereum.org/developers/docs/consensus-mechanisms/pos/rewards-and-penalties/
- [eth2book inactivity] https://eth2book.info/latest/part2/incentives/inactivity/
- [ACP-267] https://github.com/avalanche-foundation/ACPs/blob/main/ACPs/267-uptime-requirement-increase/README.md
- [Avalanche blog ACP-267] https://build.avax.network/blog/acp-267-validator-uptime-requirement
- [Cosmos SDK x/slashing] https://docs.cosmos.network/sdk/latest/api-reference/grpc/slashing
- [Secret docs] https://docs.scrt.network/secret-network-documentation/infrastructure/secret-cli/slashing
- [Cosmos forum #11783] https://forum.cosmos.network/t/eliminate-the-downtime-slash-and-reduce-downtime-jail/11783
- [SIMD-0033] https://github.com/solana-foundation/solana-improvement-documents/blob/main/proposals/0033-timely-vote-credits.md
- [Figment TVC] https://www.figment.io/insights/solanas-timely-vote-credits-reduce-vote-latency/
- [SFDP updates] https://solana.com/news/solana-foundation-delegation-program-updates
- [SFDP criteria] https://solana.org/delegation-criteria
- [Sui validator rewards] https://docs.sui.io/operators/validator/validator-rewards
- [Aptos leaderboard] https://aptos.dev/network/nodes/validator-node/verify-nodes/leaderboard-metrics
- [Staking Rewards Aptos] https://docs.stakingrewards.com/staking-data/methodologies/aptos-srb
- [ICP PBR wiki] https://internetcomputer.org/wiki/performance-based-rewards/
- [ICP remuneration] https://internetcomputer.org/wiki/node-provider-remuneration/
- [Helium denylist HIP-40] https://github.com/helium/HIP/blob/29b78e52e453bc790eca725259a818422712d65c/0040-validator-denylist.md
- [HIP-66 trust score] https://github.com/helium/HIP/blob/main/0066-trust-score-and-denylist-convenience.md
- [Helium halving 2025] https://blog.helium.com/helium-halving-2025-what-it-means-for-hotspot-operators-hnt-holders-and-network-governance-8ecaa1fff464
- [HIP-123] https://github.com/helium/HIP/blob/main/0123-redefining-data-only-onboarding-and-assertion-fees.md
- [POKT Shannon] https://chainwire.org/2025/09/16/pocket-network-completes-shannon-network-upgrade-becoming-a-cosmos-chain-with-usage-based-economics/
- [Theta Edge FAQ] https://support.thetanetwork.org/hc/en-us/articles/32047322958995-Theta-Edge-Node-FAQ
- [Theta EEN staking] https://docs.thetatoken.org/docs/elite-edge-node-staking-process
- [Peercoin docs] https://www.peercoin.net/docs/proof-of-stake
- [Pulsar arXiv 2411.14245] https://arxiv.org/pdf/2411.14245
- [Filecoin spec minting] https://spec.filecoin.io/systems/filecoin_token/minting_model/
- [Filecoin spec PoSt] https://spec.filecoin.io/algorithms/pos/post/
- [Filecoin docs proving] https://docs.filecoin.io/provide-storage/filecoin-economics/storage-proving
- [PoRep report] https://filecoin.io/proof-of-replication.pdf
- [PL research] https://research.protocol.ai/blog/2020/a-research-perspective-on-filecoin-part-two/
- [Kaspa tokenomics] https://wiki.kaspa.org/en/tokenomics
- [EF blog 2025-07-08] https://blog.ethereum.org/2025/07/08/partial-history-exp
- [eth-clients history-endpoints] https://github.com/eth-clients/history-endpoints
- [Portal specs] https://github.com/ethereum/portal-network-specs
- [Old Faithful docs] https://docs.old-faithful.net/
- [Triton report] https://docs.triton.one/project-yellowstone/old-faithful-historical-archive/old-faithful-public-report
- [Celestia Matcha blog] https://blog.celestia.org/matcha/
- [Walrus paper arXiv 2505.05370] https://arxiv.org/html/2505.05370v2
- [Walrus PoA blog] https://blog.walrus.xyz/how-walrus-proof-of-availability-works/
- [Arweave syncing-packing] https://docs.arweave.org/developers/mining/overview/syncing-and-packing
- [SPoRA post] https://arweave.medium.com/the-arweave-network-is-now-running-succinct-random-proofs-of-access-spora-e2732cbcbb46
- [Storj forum new audit scoring] https://forum.storj.io/t/new-audit-scoring-is-live/19466
- [Storj docs DQ] https://storj.dev/node/faq/why-is-my-node-disqualified
- [Sia whitepaper] https://sia.tech/whitepaper.pdf
- [Sia docs] https://docs.sia.tech/provide-storage/about-hosting-on-sia
- [Swarm mechanics] https://blog.ethswarm.org/foundation/2022/the-mechanics-of-swarm-networks-storage-incentives/
- [Redistribution.sol] https://github.com/ethersphere/storage-incentives/blob/master/src/Redistribution.sol
- [Apple WWDC21 fraud] https://developer.apple.com/videos/play/wwdc2021/10244/
- [WWDC26 App Attest] https://developer.apple.com/videos/play/wwdc2026/201/
- [Approov limits] https://approov.io/blog/limitations-of-apple-devicecheck-and-apple-app-attest
- [SEC PoW 2025-03-20] https://www.sec.gov/newsroom/speeches-statements/statement-certain-proof-work-mining-activities-032025
- [SEC staking 2025-05-29] https://www.sec.gov/newsroom/speeches-statements/peirce-statement-protocol-staking-052925
