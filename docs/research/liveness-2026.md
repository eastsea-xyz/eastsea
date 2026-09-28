# 검증자 대부분이 꺼져도 블록이 쌓이게 하려면: 각 체인이 쓰는 방식과 Aether 평가 (2026-09-28)

목표: "Mac이 한 대만 켜져 있어도 블록이 계속 쌓인다." 그러면서도 서로 충돌하는 두 확정 이력은 절대 나오지 않아야 한다.
지금의 Aether: Commonware `simplex`(BLS 임계 VRF scheme)를 쓰고 블록은 1초마다 나온다. 위원회는 3f+1석이다. 위원 1/3 이상이 꺼지면 체인이 멈추고, 돌아오면 이어서 간다(07-consensus.md "열린 위원회" 6).

## 0. 결론 먼저

- **불가능성 정리.** 동적 가용성(참여자가 몇 명이든 살아 있음)과 파티션 중 확정성을 **한 체인**이 동시에 가질 수는 없다. 증명은 Lewis-Pye & Roughgarden(Resource Pools and the CAP Theorem, 2020)과 Neu·Tas·Tse(ebb-and-flow, S&P 2021)에 있다. "한 대만 켜져도"를 만족하는 블록은 정의상 **확정되지 않은(tentative) 블록**일 수밖에 없다.
- 업계 해법은 두 갈래다. 하나는 Ethereum·Polkadot·Cardano+Peras 계열이다. 가용 체인이 계속 자라고, 그 위에 확정 가젯을 얹으며, 오래 꺼진 참여자는 천천히 빼낸다. 다른 하나는 Solana·Sui·Aptos·Cosmos·NEAR·Avalanche 계열로, 멈추고 사람이 재시작한다.
- **Aether 권고.** 메인넷(11월 초)은 (a) 멈추는 BFT에 빠른 복구와 수면 대책을 붙여 출시한다. 그다음 (b) ebb-and-flow를 프로토콜 3 후보로 단계적으로 만든다. 가용 체인은 VRF 추첨으로 켜진 등록 Mac 누구나 만들고, 지금의 simplex는 체크포인트 확정 가젯이 된다. 오래 꺼진 위원을 빼는 비상 재추첨은 14일 뒤에 하고, **마지막 확정 명단의 과반 서명**을 요구한다. (c) 최장 체인만 쓰는 방식은 권하지 않는다.
- Commonware(2026.9.0)에는 2/3 정족수 없이 블록을 만드는 모드도, 최장 체인 모드도 **없다**. 따라서 가용 체인은 Aether가 직접 구현해야 한다.

## 1. Ethereum: Gasper = LMD-GHOST(가용) + Casper FFG(확정)

**메커니즘**
- 슬롯은 12초, 에포크는 32슬롯(6.4분)이다. 제안자는 RANDAO로 뽑힌다. 증명자(attester)는 매 에포크 한 번 두 가지 투표를 동시에 한다. 헤드 투표는 LMD-GHOST 포크 선택용이고, 소스→타깃 체크포인트 투표는 FFG용이다.
- 가용 체인(LMD-GHOST)은 투표자가 몇 명이든 자란다. 제안자가 한 명뿐이어도 블록은 나온다. 확정은 FFG로 이루어진다. 체크포인트가 2/3 투표로 justified되고, 연속 두 에포크가 justified되면 finalized된다. 보통 2~3 에포크(12.8~19.2분)가 걸린다.
- 안전성은 책임 추적형이다(accountable safety). 두 확정이 충돌하면 스테이크 1/3 이상을 슬래시할 수 있다.

**비활성 누출(inactivity leak).** 출처는 eth2book과 consensus-specs(Bellatrix 값)다.
- 4에포크(`MIN_EPOCHS_TO_INACTIVITY_PENALTY`, 약 25분) 동안 확정이 없으면 누출이 켜진다.
- 비활성 점수는 놓친 에포크마다 +`INACTIVITY_SCORE_BIAS`(4)씩 오른다. 누출이 없을 때는 에포크마다 −`INACTIVITY_SCORE_RECOVERY_RATE`(16)씩 내려간다.
- 에포크당 벌금은 `s·B / (4·2^24)`다. 점수가 선형으로 오르므로 누적 손실은 **2차 함수**가 된다.
- 완전히 꺼진 32 ETH 검증자는 약 **4,686에포크(약 3주)** 만에 방출된다. 이것이 "절반이 꺼졌을 때 2/3을 되찾는 시간"의 대략적인 상한이다. 꺼진 스테이크가 1/3 아래로 줄어드는 시점에 확정이 다시 시작된다.
- **왜 느린가.** 파티션이 일어나면 양쪽이 각자 상대편을 누출시켜, 각 쪽에서 2/3을 되찾고 **둘 다 확정**할 수 있다. 이 경우 두 확정 이력이 생긴다. 이를 막는 수단은 누출 속도, 곧 몇 주라는 시간뿐이다. 그 사이에 사람이 파티션을 알아채고 사회적 합의로 한쪽을 버릴 여유를 준다. 확정이 돌아온 뒤에도 점수가 천천히 내려가는 것은 확정이 켜졌다 꺼졌다 하는 진동을 막기 위해서다.

**실제 사고**
- 2023-05-11과 05-12. 첫 번째는 확정이 약 25분 지연됐고(3~4에포크), 두 번째는 약 1시간(8~9에포크) 지연됐다. 원인은 에포크 N−2 블록을 가리키는 오래된 증명이었다. Prysm과 Teku가 이를 처리하면서 상태를 반복 재생성했고, 그 과부하로 참여율이 떨어졌다. **블록은 계속 나왔다.** Lighthouse는 해당 증명을 버리고 계속 살아 있었다. 사람의 개입 없이 회복됐고 누출 손실도 작았다. 클라이언트 다양성이 결정적이었다(Offchain Labs 사후 보고서).
- 2025-12-04 Fusaka 직후에는 Prysm v7.0.0이 오래된 증명 때문에 옛 상태를 재생했다. 42에포크 동안 참여율이 약 75%로 떨어졌고, 블록 248개가 누락됐으며(18.5%) 보상 약 382 ETH가 손실됐다. 2/3까지는 9%p만 남아 있었다. 같은 버그가 다수 클라이언트에 있었다면 확정이 멈췄을 것이다.
- 교훈: 가용 체인이 있으면 **사용자 체감 정지는 0**이다. 대신 확정 지연 동안 L2 브리지·출금·거래소 입금이 멈춘다. 이들은 확정만 믿기 때문이다.

**다음 단계**
- 3SF(D'Amato·Saltini·Tran·Zanolini, arXiv 2411.00558)는 3슬롯 만에 확정한다. SSF보다 기대 확정 시간이 약 46% 길지만 구현할 수 있다. 가용 부분은 TOB-SVD나 RLMD-GHOST 계열이 맡는다.
- Orbit SSF는 큰 검증자 집합에서 작은 위원회를 천천히 회전시키며 뽑는다. Aether의 추첨·교체 속도 제한과 같은 발상이다.
- RLMD-GHOST(eprint 2023/279)는 최근 η 슬롯의 투표만 센다. 표가 만료되므로 참여율이 변해도 살아 있고, 제한된 비동기에도 견딘다.
- Goldfish는 표를 1슬롯 만에 만료시킨다. 동적 가용성은 있지만 짧은 비동기에도 이미 확인된 블록이 뒤집힐 수 있어 실전용으로는 약하다.
- TOB-SVD는 투표 한 라운드로 결정하며 1/2 미만의 적대자를 견딘다.
- 현황(2026): lean consensus(구 beam chain) 트랙에서 3SF를 명세하고 있고, 메인넷 일정은 확정되지 않았다.

## 2. Polkadot: BABE(→SASSAFRAS/SAFROLE) + GRANDPA

- **BABE**는 6초 슬롯의 VRF 추첨 방식이다. 보조 슬롯이 있어 빈 슬롯을 라운드로빈으로 메운다. 포크 선택은 "가장 긴 체인 중 확정 블록을 포함하는 것"이다. 확정과 무관하게 블록 생산이 계속된다.
- **SASSAFRAS(SAFROLE)**는 링 VRF로 단일 비밀 리더를 뽑는다. 슬롯당 블록이 정확히 1개라 포크가 거의 없다. JAM에 채택됐고, Polkadot 릴레이 체인 적용은 개발 중이다.
- **GRANDPA**는 블록이 아니라 **체인(prefix)** 에 투표한다. 2/3이 동의한 가장 높은 공통 조상을 한 번에 확정하므로, 멈췄다가 돌아오면 쌓인 수천 블록을 한 라운드에 확정한다. 이것이 ebb-and-flow의 "따라잡기"다.
- **사고: Kusama 2024-02-15.** 약 1시간씩 두 번 확정이 멈췄다. 원인은 비활성화된 검증자가 제기한 분쟁(dispute)이 잘못 "Active"로 남은 것이었다. 체인 선택이 그 분쟁 블록을 제외하면서 확정이 막혔다. 확정 지연이 **500블록**에 이르자 "오래된 분쟁 무시" 안전망이 작동해 자동으로 회복됐다. 초기 2019년 Kusama에도 GRANDPA가 멈춘 적이 있는데, 이때는 거버넌스와 재시작으로 풀었다(Gavin Wood, "Kusama's First Adventure"). Substrate에는 확정이 멈추면 BABE 생산 속도를 늦추는 **unfinalized slack/backoff** 규칙이 있다. 확정되지 않은 블록이 무한히 쌓이는 것을 막는 장치다.

## 3. Cardano, Algorand, Avalanche

**Ouroboros Praos**
- 1초 슬롯에서 각 풀이 VRF로 자신이 리더인지 비공개로 확인한다. 활성 슬롯 계수 f=0.05이므로 블록 간격은 평균 20초다. 최장 체인 규칙을 쓰고, 롤백은 최대 k=2160블록(약 12시간)까지만 허용한다. 체인 밀도가 떨어져도 계속 자라므로 동적 가용성이 있다. 대신 확정은 확률적이다. 거래소는 흔히 15~30블록 이상을 기다린다.
- **Genesis**는 새로 합류하거나 오래 꺼졌던 노드를 위한 규칙이다. k 이상 갈라진 체인들 가운데 갈림점 직후 구간의 **밀도**가 높은 쪽을 고른다. 노드가 긴 가짜 체인에 속지 않게 한다. 노트북처럼 자주 자는 노드에 특히 중요하다.
- **Peras**는 풀 위원회가 라운드마다 투표하고, 인증서를 받은 블록에 큰 가중치(boost)를 준다. 이렇게 수 분 안에 "높은 확신"을 얻는다. 위원회가 모이지 않으면 **쿨다운**에 들어가 순수 Praos로 돌아간다. 즉 확정 계층이 실패해도 체인은 멈추지 않는다. 일정은 Dijkstra 하드포크(PV12)로 2026년 4분기에 코드 완료, 2027년 2분기 활성화 목표다.
- **사고: 2025-11-21 메인넷 분할.** 조작된 위임 tx가 2022년부터 있던 역직렬화 버그를 건드렸다. 신버전 노드는 이 tx를 받아들였고 구버전 노드는 거부해서 두 체인이 병행했다. **양쪽 모두 블록을 계속 만들었다.** 약 14시간 뒤 SPO들이 10.5.3으로 올리면서 최장 체인 규칙에 따라 한쪽으로 수렴했고, 독이 든 체인에만 들어간 tx는 사라졌다. 교훈: 가용 체인은 멈추지 않는다. 대신 확정 없는 구간의 tx는 **되돌려질 수 있다**. 사용자 화면에서는 이것이 "재제출 필요"로 나타난다.

**Algorand**: 위원회를 VRF로 뽑고 BA*로 매 라운드 즉시 확정한다. 정족수가 안 되면 블록도 없다. 2019년 제네시스 이후 메인넷이 멈춘 적은 없다고 주장한다. 2022-07-08 테스트넷은 약 5시간 멈췄다. 블록 검증 캐시 버그로 유효한 tx가 무효 판정을 받았고, 수정 뒤 재개됐으며 포크는 없었다. 파티션이 오면 멈췄다가 "recovery" 기간 로직으로 합류한다. 안전 우선이다.

**Avalanche Snowman**: 반복 부표본 투표로 블록을 수락한다. 정족수 α를 k 샘플 안에서 확보하지 못하면 수락이 멈춘다. 2024-02-23에는 P·X·C 체인이 약 2시간 멈췄다. 인스크립션 폭주가 가십과 멤풀 관리 버그를 건드렸고, 패치와 재시작으로 복구했다.

## 4. NEAR: Doomslug + 확정 가젯

- 높이 h의 생산자는 직전 블록에 대한 승인(endorsement)이나 건너뛰기(skip)를 **스테이크 2/3 초과** 만큼 모아야 블록을 낼 수 있다(nomicon Consensus). 따라서 1/3 이상이 꺼지면 **생산도 멈춘다**. "Doomslug 확정"은 블록 하나 뒤에 나오는 실용적 확정이다. BFT 확정은 연속 높이 두 블록이 위에 올라오면 성립한다.
- 복구 수단은 에포크(약 12시간)마다 가동률이 낮은 검증자를 쫓아내는 킥아웃이다. 에포크 경계를 넘길 만큼 체인이 살아 있어야 작동하므로 대규모 동시 이탈에는 소용이 없다. NEAR는 사실상 멈추는 BFT 계열이다.

## 5. 멈추는 BFT 체인과 그 복구

**Solana(TowerBFT, 2/3 투표 잠금)**
- 2021-09-14에 약 17시간 멈췄다. IDO 봇 tx 폭주로 메모리가 고갈됐고, 운영자들이 합의해 스냅샷에서 재시작했다.
- 2022-04-30 약 7시간(NFT 민팅 봇), 2022-06-01 약 4.5시간(durable nonce 버그), 2022-10-01 약 6시간(중복 노드 설정 오류), 2023-02-25 약 19시간(거대 블록이 turbine 전파를 막음).
- 2024-02-06 약 5시간: LoadedPrograms JIT 캐시 버그로 합의가 한 블록에 멈췄다(Anza 보고서).
- **복구 절차.** 운영자들이 최고 optimistically confirmed 슬롯(예: 246,464,040)을 정하고 거기서 스냅샷을 만든다. 그다음 패치 바이너리를 배포하고 스테이크 80% 이상이 모일 때까지 수동으로 재시작한다(`--wait-for-supermajority`). 확정 이력은 한 번도 갈라지지 않았다. 멈춤을 대가로 안전을 지킨 것이다.
- 2024년 이후로는 약 30개월 넘게 전체 장애가 없다. 차세대 합의인 Alpenglow(Votor/Rotor)는 20+20 설계다. 80% 투표면 한 라운드에 확정되고 60%면 두 라운드에 확정된다.

**Sui(Narwhal/Bullshark → Mysticeti DAG BFT)**
- 2024-11-21에 약 2.5시간 멈췄다. 혼잡 제어 코드의 assert!가 추정 비용 0에서 검증자를 크래시시켰다. 2026-01에는 약 6시간 합의가 멈췄다. 2026-05-28에는 세 번째 큰 정지가 있었는데, v1.72 가스 과금 버그에 임시 패치가 DKG 난수 결함을 노출시킨 경우였다.
- 복구는 모두 핫픽스 배포 후 재시작이었다. Sui 재단은 이후 대책으로 "safe mode"(재구성 경로의 우아한 저하)와 "크래시 유발 입력 건너뛰기"를 꼽았다. 다시 말해 **멈추지 않게 하는 것은 합의 모드가 아니라 결정적 버그 격리**라는 결론이다.

**Aptos(Jolteon/DiemBFT → Raptr 계열)**: 2023-10-19에 약 5시간 멈췄다. 코드 업데이트 문제였고, 핫픽스와 재시작으로 복구했다.

**CometBFT/Tendermint**: +2/3 prevote와 precommit이 없으면 높이가 올라가지 않는다. Cosmos Hub, Terra, Osmosis 등이 업그레이드 실패나 앱 해시 불일치로 멈췄을 때는 운영자들이 공지된 높이에서 함께 재시작했다.

**이들이 멈춤을 택한 이유**
- (1) 즉시 확정되므로 거래소와 브리지가 단순해진다(1블록 = 끝).
- (2) 가용 체인과 포크 선택, 재편(reorg) 처리가 없어서 코드와 감사 표면이 절반으로 준다.
- (3) 실제 장애는 대부분 **결정적 버그**다. 모든 노드가 같은 입력에 같이 죽는다. 이런 경우에는 가용 체인이 있어도 버그 블록을 넘지 못한다. Solana·Sui·Aptos·Avalanche 사고가 모두 그랬다. 가용 체인이 도움 되는 경우는 Ethereum 2023·2025처럼 **일부 클라이언트나 일부 노드만** 꺼지는 경우다.

### 5.1 사고 한눈에 보기

| 날짜 | 체인 | 증상 | 기간 | 블록 생산 | 복구 방식 |
|---|---|---|---|---|---|
| 2021-09-14 | Solana | tx 폭주, 메모리 고갈 | 약 17시간 | 멈춤 | 운영자 합의 재시작 |
| 2022-07-08 | Algorand 테스트넷 | 검증 캐시 버그 | 약 5시간 | 멈춤 | 패치 후 재개 |
| 2023-02-25 | Solana | 거대 블록 전파 실패 | 약 19시간 | 멈춤 | 다운그레이드와 재시작 |
| 2023-05-11·12 | Ethereum | 오래된 증명 과부하 | 25분, 1시간 | **계속** | 자동(클라이언트 다양성) |
| 2023-10-19 | Aptos | 코드 업데이트 문제 | 약 5시간 | 멈춤 | 핫픽스와 재시작 |
| 2024-02-06 | Solana | JIT 캐시 버그 | 약 5시간 | 멈춤 | 확인된 슬롯에서 재시작 |
| 2024-02-15 | Kusama | 분쟁 교착 | 약 1시간×2 | **계속** | 500블록 안전망이 자동 해소 |
| 2024-02-23 | Avalanche | 가십·멤풀 버그 | 약 2시간 | 멈춤 | 패치와 재시작 |
| 2024-11-21 | Sui | 혼잡 제어 assert | 약 2.5시간 | 멈춤 | 핫픽스 |
| 2025-11-21 | Cardano | 역직렬화 버그로 체인 분할 | 약 14시간 | **양쪽 계속** | 노드 업그레이드, 최장 체인 수렴 |
| 2025-12-04 | Ethereum | Prysm 옛 상태 재생 | 42에포크 | **계속** | 플래그 우회 후 패치(확정 유지) |
| 2026-01, 2026-05-28 | Sui | 합의 정지, 가스 버그 | 약 6시간 등 | 멈춤 | 핫픽스 |

이 표에서 읽을 수 있는 것:
- 가용 체인이 있는 체인(Ethereum, Kusama, Cardano)은 사고 중에도 블록이 쌓였다.
- 멈추는 BFT 체인은 모두 사람이 조율한 재시작으로 복구했다. 확정 이력이 갈라진 사례는 없다.
- Cardano 사례는 가용 체인의 대가를 보여 준다. 확정되지 않은 구간의 tx는 사라질 수 있다.

## 6. 학술 정리

| 모델/프로토콜 | 요지 | Aether 관련성 |
|---|---|---|
| Sleepy model(Pass & Shi 2017) | 참여자가 임의로 자고 깨도, 깨어 있는 정직한 쪽이 과반이면 안전하고 살아 있다(동기 가정) | "노트북이 잠든다"를 정식화한 모델 |
| Snow White(Daian·Pass·Shi, FC 2019) | 수면 모델 PoS 최장 체인. 체크포인트로 장기 공격을 막는다 | (c) 방식의 원형 |
| Ebb-and-flow(Neu·Tas·Tse, S&P 2021) | 가용 원장 LOG_da와 그 접두사인 확정 원장 LOG_fin. 파티션 중에는 확정만 뒤처지고 회복하면 따라잡는다 | (b) 방식의 이론적 근거 |
| 가용성-책임성 딜레마, accountability gadget(Neu·Tas·Tse 2021~22) | 동적 가용성과 책임 추적 안전성은 한 원장에서 양립할 수 없다. 가젯을 덧대는 방식으로 해결 | BLS 임계 서명은 누가 서명했는지 알 수 없다 → 책임 추적에 불리 |
| Resource pools / CAP(Lewis-Pye·Roughgarden) | unsized(참여 규모를 모름) + 부분 동기 환경에서는 적응성과 확정성이 공존할 수 없다 | 불가능성 정리 |
| Goldfish(2022), RLMD-GHOST(2023), TOB-SVD(2024) | 표 만료로 동적 가용성과 빠른 확인을 얻는다. 비동기 복원력과는 트레이드오프 | 가용 체인의 포크 선택 후보 |
| Accountable safety implies finality(2023) | 책임 추적 안전성이 있으면 확정성이 따라 나온다 | 확정 가젯 설계 근거 |

핵심은 이렇다. **"켜진 쪽만으로 확정"을 허용하는 순간 파티션 양쪽이 각자 확정할 수 있다.** 타협은 두 가지뿐이다. (i) 시간으로 막는다(Ethereum 누출 3주 + 사회적 개입). (ii) 더 큰 집합의 정족수 교집합으로 막는다(뒤의 권고).

## 7. Commonware: 무엇이 있고 무엇이 없나

로컬 레지스트리의 `commonware-consensus-2026.9.0`과 GitHub monorepo를 확인했다.
- 모듈은 `simplex`, `marshal`, `aggregation`, `types`뿐이다. `ordered_broadcast`는 2026-08-19에 **제거됐다**(#4536, "[consensus] Remove ordered broadcast").
- **simplex**에서는 notarize, nullify, finalize가 **모두 2f+1** 을 요구한다. 리더가 제안해도 2f+1 notarize가 없으면 다음 뷰로 가지 못하고 nullify도 2f+1이 필요하다. 그러니 1/3 이상이 꺼지면 뷰가 전진하지 않는다. `SkipPolicy`(Aether는 5초로 사용 중)는 비활성 리더의 타임아웃을 0으로 줄여 주지만, 이것도 "정족수는 살아 있고 리더만 죽은" 경우에만 도움이 된다.
- **Minimmit**(`pipeline/minimmit`, arXiv 2508.10862)은 n=5f+1에서 **40% 정족수(M)** 로 뷰를 전진·공증하고 **80%(L)** 로 확정한다. 켜진 비율이 40~80%일 때는 공증된 블록이 계속 쌓이고 확정만 멈춘다. 가용 체인을 부분적으로 흉내 내는 셈이다. 대신 비잔틴 허용치가 20%로 떨어진다. 현재는 명세와 Quint 모델뿐이고 **크레이트는 출시되지 않았다**. 40% 아래로 떨어지면 역시 멈춘다.
- **aggregation**은 외부에서 순서가 정해진 항목(상태 루트 등)에 정족수 인증서를 비동기로 붙이는 모듈이다. 가용 체인 위에 "확정 체크포인트 인증서"를 붙이는 부품으로 재활용할 여지가 있다. 이 역시 2f+1 기반이다.
- 결론: Commonware에는 최장 체인, 가용 체인, 비활성 누출이 **없다**. (b)를 하려면 가용 체인(생산, 전파, 포크 선택, 재편)을 Aether가 직접 만들고, simplex는 체크포인트만 확정하는 가젯으로 써야 한다.

**Aether 로컬 코드(읽기만 함)**
- `crates/node/src/engine.rs`: `Consensus<…, aether_light::Elector, …, Marshaled>`. simplex 블록 자체가 실행 블록이다. `Deferred`(marshal)가 인증 후 실행하고, `ScheduleEpocher`가 3,600블록 에포크를 정한다. `leader_timeout` 2초, `skip_timeout` 5초, `activity_timeout` 20뷰(`main.rs:1209~1214`).
- `crates/node/src/application.rs`: `propose`/`verify`는 조상(ancestry)에 대해 실행하고 검증한다. 상태는 `Update::Block`, 곧 **확정 시에만** 반영된다. 지갑에는 확정 상태만 보인다.
- 설계 문서 07 "ebb-and-flow — 미구현"과 12-launch-plan C5의 "대규모 이탈 시 비확정 임시 체인(연구)"가 이 보고서가 다루는 공백이다.
- 중요한 제약이 하나 있다. 재공유와 교체에는 **현 위원회 2f+1의 서명**이 필요하다(`handoff.rs`). 위원 1/3 이상이 영구히 사라지면 교체조차 할 수 없다. 이 경우 임계 키의 identity도 되살릴 수 없다(share 임계치 미달). 지갑이 고정 신뢰하는 identity가 끝나는 것이다. 지금 구조에서 이 경우의 복구는 **새 DKG와 새 identity, 곧 사회적 재시작**뿐이다.

## 8. Aether 선택지 평가

전제: 블록은 1초, 위원회는 적격 Mac의 1/4(3f+1, 4~128석)이다. 적격 조건이 가동률 95% 이상이라 위원은 대체로 상시 켜진 Mac이지만, 노트북 덮개를 닫는 순간 즉시 잠든다.

### (a) 멈추는 BFT 유지 + 빠른 복구

- **파티션 안전성**: 최상이다. 2f+1 교집합이 보장되므로 확정 충돌이 불가능하다(f<1/3 가정).
- **지갑·거래소**: 바꿀 것이 없다. 1블록 = 확정.
- **Commonware 위 복잡도**: 낮다. 이미 구현돼 있다.
- **잠드는 노트북 적합성**: 나쁘다. 위원 1/3이 동시에 자면 멈춘다. 완화책은 다음과 같다.
  - 위원 선출 시 AC 전원과 데스크톱을 우선한다. 노트북은 "잠자기 방지 약속"을 요구한다(`IOPMAssertionCreateWithName(PreventSystemSleep)`, 전원 연결 시만).
  - 뷰 누락률이 높은 위원을 다음 추첨에서 우선 교체한다.
  - 위원을 키운다(수면 사건이 독립이면 1/3 동시 수면 확률은 위원 수에 따라 지수적으로 준다).
- **남는 구멍**: 1/3 이상이 영구 이탈하면 identity를 잃고 수동으로 재시작해야 한다.
- **공수**: 2~3주(수면 방지, 선출 가중, 재시작 런북·도구, 모니터링).

### (b) ebb-and-flow: 가용 체인 + simplex 확정 가젯 + 느린 재추첨

구조:
- **가용 체인(LOG_da).** 매 슬롯(1초) 각 적격 등록 Mac이 `VRF(seed_epoch, slot) < τ`로 리더인지 확인한다. Praos와 같은 비공개 추첨이다. seed는 마지막 확정 체크포인트의 BLS 임계 시드를 k 슬롯 지연해서 쓴다. 이렇게 해야 참여자가 줄면 블록률이 비례해서 떨어지고, 한 대만 켜져 있어도 블록이 나온다.
- **포크 선택.** 마지막 확정 체크포인트에서 출발한 체인들 가운데 가장 무거운 것을 고른다. 무게는 "서로 다른 생산자 수"이고, 동점이면 VRF 값이 작은 쪽이다. 자는 노트북이 깨어났을 때는 Genesis식 밀도 규칙을 쓴다.
- **확정 가젯(LOG_fin).** 지금의 simplex를 그대로 쓰되, 블록 내용을 "가용 체인 tip 해시 + 높이"(체크포인트)로 바꾼다. 확정되면 그 조상 전부가 한꺼번에 확정된다(GRANDPA식). 위원회가 돌아오면 쌓인 prefix를 한 번에 확정한다.
- **실행.** 가용 체인에서 낙관적으로 실행하고 결과를 "제안됨"으로 표시한다. 확정 상태는 체크포인트 기준이다. 지금 `Update::Block`에서 하던 확정 반영이 체크포인트 수신으로 옮겨 간다.

평가:
- **파티션 안전성**: 확정 원장은 (a)와 같다. 가용 원장은 파티션 양쪽에서 각자 자라다가, 회복되면 한쪽이 **재편**된다(Cardano 2025-11과 같은 양상).
- **확정 복구와 느린 재추첨.** 확정이 T_leak 동안 없으면 가용 체인이 "비상 재추첨"을 발동한다. 그러나 이 재추첨이 파티션 양쪽에서 동시에 일어나면 확정 충돌이 생긴다. 그래서 다음 조건을 둔다.
  - 재추첨 트랜잭션은 **마지막 확정 명단(적격 Mac 전체, 위원회만이 아님) 중 과반**의 ed25519 서명을 모아야 유효하다. 두 파티션이 동시에 과반을 가질 수는 없으므로 정직한 다수 가정 아래 충돌 확정이 불가능하다. 적격 집합은 위원회의 약 4배이므로 1/2 이탈은 위원회 1/3 이탈보다 훨씬 드물다.
  - 새 위원회는 새 DKG로 **새 identity**를 만든다. 지갑은 "옛 identity 마지막 확정 체크포인트 + 과반 서명 인계 증명"을 검증해 새 identity로 넘어간다(light 크레이트 확장).
  - 비잔틴 참여자가 있는 상태에서 파티션이 겹치면 과반 교집합이 뚫릴 수 있다. 따라서 T_leak를 14일로 길게 잡아 사회적 개입의 여지를 둔다. Ethereum의 3주와 같은 논리다.
- **지갑·거래소**:
  - 지갑은 세 상태 표시가 필요하다(07에 예정된 "제안됨 / 확정됨 / 증명됨"). 확정 전 상태는 되돌려질 수 있다는 문구를 붙인다.
  - 거래소 입금, 브리지, 에이전트 결제 한도 소진은 **확정만** 인정한다.
  - RPC에 `finalized`와 `latest`를 분리한다(Ethereum JSON-RPC 관례).
- **Commonware 위 복잡도**: 높다. 가용 체인 p2p 전파, 포크 선택, 재편 시 상태 되감기(저장소 스냅샷·저널), VRF 추첨(적격 Mac 키로 ECVRF, RFC 9381), 체크포인트 확정과의 결합, 비상 재추첨, identity 인계를 모두 새로 만들어야 한다. marshal·Deferred의 "확정 = 실행" 가정도 풀어야 한다.
- **잠드는 노트북 적합성**: 최고다. 켜진 Mac 누구나 생산하고, 확정은 위원회가 돌아오면 따라잡는다.
- **공수**: 핵심 구현 3~4개월, 거기에 Quint 명세, 시뮬레이션, 감사 1~2개월.

### (c) Ouroboros식 최장 체인만

- **파티션 안전성**: 확률적 확정만 있다. 파티션 뒤 긴 재편이 가능하고 롤백 상한 k 밖의 보장이 없다. 책임 추적형 확정이 없다.
- **지갑·거래소**: 확인 수 기반(k블록)으로 전면 재설계해야 한다. 1초 블록에서 Praos와 비슷한 보장을 얻으려면 수백~수천 블록을 기다려야 한다. 지금의 "인증서 하나로 잔액 검증"하는 경량 클라이언트와 ZK 경로도 무너진다.
- **복잡도**: (b)의 가용 체인 부분과 같지만, simplex와 임계 identity라는 기존 자산을 버리게 된다.
- **노트북 적합성**: 좋다. 대신 DeviceCheck 시빌 모델에서 공격자가 **켜진** Mac의 과반을 잡으면 재작성이 가능하다. 밤에 켜진 Mac이 적은 시간대가 취약하다.
- **공수**: 3~4개월에 경량 클라이언트·ZK 재설계가 더해진다. **비권장.**

### 비교 요약

| | (a) 멈춤+복구 | (b) ebb-and-flow | (c) 최장 체인 |
|---|---|---|---|
| 한 대만 켜져도 블록 | ✗ | ✓(임시) | ✓(확률적) |
| 파티션 중 확정 충돌 | 불가 | 불가(과반 재추첨 조건 시) | 해당 없음(확정 없음) |
| 지갑·거래소 변경 | 없음 | 3상태, finalized RPC | 전면 |
| 경량 클라이언트·ZK | 유지 | 유지(체크포인트) | 재설계 |
| 공수 | 2~3주 | 4~6개월 | 4~6개월+ |

## 9. 권고와 구체 파라미터

**권고: 메인넷은 (a)로 출시하고, (b)는 프로토콜 3 후보로 단계적으로 개발한다. (c)는 채택하지 않는다.**
근거는 세 가지다. (1) 업계 실사고 대부분은 결정적 버그였고, 가용 체인도 이를 막지 못했다. (2) (b)의 이득은 "일부만 꺼짐", 즉 수면·회선 문제에 집중된다. 이것이 바로 Aether의 주 위험(노트북)이므로 장기적으로는 필요하다. (3) 11월 메인넷까지 4~6개월짜리 합의 변경을 감사까지 마칠 수는 없다.

**(b) 파라미터 초안**

| 항목 | 값 | 근거 |
|---|---|---|
| 저하 모드 진입 | 확정 없는 시간 10초(10슬롯) | simplex 정상 확정은 1초 안팎 |
| 가용 체인 슬롯 | 1초, 활성 계수 τ로 **켜진 전원 기준** 기대 리더 수 ≈ 1 | 켜진 비율이 p면 블록률은 약 p |
| 추첨 시드 | 마지막 확정 BLS 시드, 2에포크 지연 | 리더 선취 방지(07 주의사항) |
| 가용 블록 가스 한도 | 정상의 1/4 | 재편 비용, 되감기 부담 축소 |
| 확정 전용 연산 | 브리지 출금, 레지스트리·인계, 업그레이드, 한도 큰 에이전트 결제 | 되돌려지면 안 되는 것 |
| 지갑 표시 | "제안됨(n블록 쌓임, 되돌려질 수 있음)" → "확정됨" → "증명됨(ZK)" | 07 3상태 |
| 거래소 권고 | `finalized` 태그만 입금 인정 | Ethereum 관례 |
| 최대 재편 깊이(정상) | 확정 체크포인트 아래로는 0(불가) | 가젯 규칙 |
| 최대 비확정 길이 | 3,600블록(1에포크) 넘으면 생산 속도를 절반씩 낮춤(최저 1블록/8초) | Substrate unfinalized backoff |
| 비상 재추첨 시작 | 확정 없는 **14일** | Ethereum 약 3주, Aether 14일 워밍업과 정렬 |
| 재추첨 조건 | 마지막 확정 적격 명단의 **>1/2** ed25519 서명, 그리고 서명자가 최근 24에포크 가용 체인에 생존 신호를 남겼을 것 | 파티션 양쪽 동시 불가 |
| 재추첨 결과 | 새 DKG, 새 identity, 옛 identity 마지막 체크포인트에 대한 인계 증명 | identity 복구 불가 문제 |
| 오래 꺼진 위원 처리(평시) | 한 에포크에 뷰 30% 이상 놓치면 다음 추첨에서 우선 교체 | 누출의 평시 버전 |

**단계별 계획**
1. **P0(지금~메인넷, 2~3주, (a))**:
   - 위원 수면 방지(전원 연결 시 `PreventSystemSleep`, 배터리 모드 위원 경고).
   - 추첨 가중(데스크톱·AC 우선)과 뷰 누락 기반 우선 교체.
   - 재시작 런북과 `aether` 도구: 마지막 확정 높이 합의, 새 DKG, 새 identity 배포.
   - 확정 지연 알림 모니터링.
   - 테스트: 위원 1/3 수면 → 멈춤 → 재개. 1/3 영구 이탈 → 런북 복구.
2. **P1(1~2개월)**: 지갑 3상태 UI와 RPC `latest`/`finalized` 분리(Minimmit이나 (b) 어느 쪽으로 가도 필요하다). `specs/consensus.qnt`에 ebb-and-flow 모델과 불변식(확정 단조성, LOG_fin ⊆ LOG_da, 재추첨 과반 교집합)을 작성한다.
3. **P2(2~3개월)**: devnet 기능 플래그로 가용 체인(VRF 슬롯, 포크 선택, 상태 되감기)을 만들고 simplex를 체크포인트 가젯으로 전환한다. 결정적 런타임으로 수면·파티션을 시뮬레이션한다. "한 대만 켜진 상태로 1시간 → 위원회 복귀 → 일괄 확정"을 통합 테스트로 확인한다.
4. **P3(1~2개월)**: 비상 재추첨(14일, 과반)과 identity 인계를 경량 클라이언트에 넣고, 외부 감사 후 프로토콜 3로 활성화한다.
5. **추적 항목**: Commonware Minimmit 크레이트가 출시되면 40~80% 구간은 Minimmit만으로 덮을 수 있다. 비잔틴 20% 한계를 감수할지 재평가한다.

## 10. 위험과 열린 질문

1. **시빌과 가용 체인.** 저하 모드에서는 **켜진** 적격 Mac의 과반이 가용 체인을 좌우한다.
   - 새벽에 켜진 Mac이 적을 때 공격자 한 명이 Mac 몇 대로 임시 체인을 재작성할 수 있다.
   - 확정 경계는 넘지 못하므로 피해는 "제안됨" 구간에 한정된다.
   - 완화: 적격 조건(가동률 95%, streak)을 가용 체인 생산자에게도 똑같이 적용한다.
2. **재편과 상태 저장소.** 지금 저장소는 확정 상태만 쓴다. 가용 체인에는 되감기 가능한 상태가 필요하다.
   - 방법은 두 가지다. 블록별 상태 차분 저널을 남기거나, 확정 기준 상태에 임시 오버레이를 얹는다.
   - history root, 에라, 가지치기(8cd426b 커밋의 저장소 계획)와 맞물린다.
3. **ZK 증명(증명됨 상태).** 증명은 확정 체크포인트 기준으로만 만든다. 임시 블록을 증명하면 재편 때 작업이 낭비된다.
4. **보상.** 노드 보상(매시간 비콘, 운영자당 1/16 상한)은 확정 체인 기준으로만 지급한다.
   - 임시 체인에서 받은 비콘은 확정되면 인정한다.
   - 파티션 양쪽에서 이중으로 보상받지 않게 한다.
5. **BLS 임계 서명과 책임 추적.** 임계 서명은 누가 서명했는지 드러내지 않는다.
   - 충돌 확정이 생겨도 범인을 특정할 수 없다. 스테이크가 없으니 슬래시도 없다.
   - 따라서 Aether의 안전은 정직한 다수 가정과 DeviceCheck에 기댄다.
   - 부분 서명 로그를 보존하면 사후 추적은 가능하다.
6. **identity 연속성.** 비상 재추첨은 identity를 바꾼다. 번들된 network.json을 고정 신뢰하는 지갑에는 인계 증명 검증 로직이 먼저 배포돼 있어야 한다. 이 로직은 P1에 포함하는 편이 안전하다.
7. **Minimmit 대안.** n=5f+1, M=40%, L=80% 구조는 부분 가용성(40% 이상 켜짐)을 합의 한 겹으로 얻는다.
   - 출시되면 (b)의 가용 체인 없이도 "절반이 자도 블록은 쌓임"을 얻을 수 있다.
   - "한 대만 켜져도"는 여전히 불가하고, 비잔틴 허용치가 33%에서 20%로 떨어진다.

## 출처

- Ethereum: [eth2book 비활성 누출](https://eth2book.info/latest/part2/incentives/inactivity/), [consensus-specs](https://github.com/ethereum/consensus-specs), [Offchain Labs 2023-05-11 사후 보고서](https://medium.com/offchainlabs/post-mortem-report-ethereum-mainnet-finality-05-11-2023-95e271dfd8b2), [CoinDesk 2023-05-17](https://www.coindesk.com/tech/2023/05/17/ethereums-loss-of-finality-what-happened), [Prysm Fusaka 사후 보고서 요약](https://crypto.news/what-broke-ethereums-fusaka-upgrade/), [3SF 논문](https://arxiv.org/abs/2411.00558), [3SF ethresear.ch](https://ethresear.ch/t/3-slot-finality-ssf-is-not-about-single-slot/20927), [RLMD-GHOST](https://eprint.iacr.org/2023/279.pdf), [SoK Speedy Secure Finality](https://arxiv.org/abs/2512.20715)
- Polkadot: [Kusama 2024-02-15 사후 보고서](https://forum.polkadot.network/t/finality-stall-on-kusama-15-02-2024-post-mortem/6398), [Kusama's First Adventure](https://medium.com/polkadot-network/kusamas-first-adventure-2cd4f439a7a4), [GRANDPA](https://polkadot.com/blog/polkadot-consensus-part-2-grandpa/), [Sassafras](https://research.web3.foundation/Polkadot/protocols/block-production/Sassafras-Part-1)
- Cardano: [Praos 기초](https://ouroboros-consensus.cardano.intersectmbo.org/docs/references/miscellaneous/cardano_praos_basics/), [Peras](https://www.iog.io/news/ouroboros-peras-the-next-step-in-the-journey-of-cardano-s-protocol-1), [2025-11 분할 보고서](https://intersectmbo.org/news/incident-report-network-partition-analysis-and-resolution-strategy), [Cardano Foundation](https://cardanofoundation.org/blog/november-2025-cardano-shows-resilience), [Dijkstra 일정](https://www.unlock-bc.com/en/cardano-dijkstra-hard-fork-sets-20262027-roadmap)
- Algorand·Avalanche·NEAR: [Algorand 테스트넷 정지](https://forum.algorand.co/t/testnet-stall-7-8-2022-post-mortem/7416), [Avalanche 2024-02](https://www.theblock.co/post/278816/avalanche-block-finalization-stall), [NEAR Consensus 명세](https://nomicon.io/ChainSpec/Consensus)
- BFT 정지: [Solana 2024-02-06 보고서](https://solana.com/news/02-06-24-solana-mainnet-beta-outage-report), [Helius Solana 장애 전사](https://www.helius.dev/blog/solana-outages-complete-history), [Sui 2024-11](https://blog.sui.io/sui-mainnet-outage-resolution/), [Sui 3회 정지 분석](https://thedefiant.io/news/blockchains/sui-blames-triple-mainnet-halt-gas-charging-bug-known-risk-patch), [Aptos 2023-10](https://www.theblock.co/post/258318/aptos-network-issues-five-hours)
- 이론: [Ebb-and-Flow](https://arxiv.org/abs/2009.04987), [Decentralized Thoughts 해설](https://decentralizedthoughts.github.io/2020-11-01-ebb-and-flow-protocols-a-resolution-of-the-availability-finality-dilemma/), [Resource Pools and the CAP Theorem](https://arxiv.org/abs/2006.10698), [Accountable Safety Implies Finality](https://arxiv.org/abs/2308.16902)
- Commonware: [simplex 문서](https://docs.rs/commonware-consensus/latest/commonware_consensus/simplex/index.html), [Minimmit 논문](https://arxiv.org/abs/2508.10862), [threshold simplex 블로그](https://commonware.xyz/blogs/threshold-simplex), [reshare 블로그](https://commonware.xyz/blogs/reshare), monorepo 커밋 #4536(ordered broadcast 제거, 2026-08-19), 로컬 `commonware-consensus-2026.9.0/src/{simplex,aggregation}/mod.rs`
