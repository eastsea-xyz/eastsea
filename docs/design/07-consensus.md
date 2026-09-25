# 07. 합의 계층

## 엔진: Commonware simplex (BLS 임계 scheme) — 구현됨 (2026-09-26)

- 인증서: `bls12381_threshold::vrf::Scheme<ed25519::PublicKey, MinSig>`. 정족수(2f+1)의 부분 서명을 모아 **그룹 공개키 하나로 검증되는 서명 하나**를 만든다. 검증자 4명 기준 확정 인증서 237B(ed25519 다중 서명, 검증자당 64B 증가) → 131B(고정).
- 지갑·경량 클라이언트는 검증자 목록 대신 위원회 identity(G2 공개키 96B)만 신뢰한다(`ValidatorSet::new(identity)`, `certificate_verifier`). 재공유(reshare)해도 identity는 유지된다.
- 리더 선출: 직전 라운드 VRF 시드(라운드에 대한 임계 서명)로 무작위 선출(`Random` V1). 라운드로빈과 달리 다음 리더를 미리 알 수 없어 표적 DoS가 어렵다.
- 키: **DKG 구현됨** (`aether dkg`, `crates/node/src/dkg.rs`). Joint-Feldman(Commonware `feldman_desmedt`), 검증자 전원이 딜러이자 플레이어. 누구도 그룹 비밀키를 알지 못하고 각자 자기 share만 갖는다.
  - 메시지는 인증·암호화된 p2p(검증자 링크)로. 딜링은 받을 때까지 재전송, 중복 딜링에는 보냈던 ack를 재전송(ack 유실 대비).
  - 제네시스 규칙: 딜러 로그 전부 필요, 로그는 받은 노드가 한 번 중계, 서로 다른 로그 두 개에 서명한 딜러는 모두가 제외, 끝에 각자 계산한 identity를 공지해 **전원 일치할 때만** 성공.
  - 결과: `<data>/threshold.json`(비밀 share, 권한 600) + `network.json`(공개 identity). 노드는 threshold.json이 있으면 그 키로, 없으면 devnet 딜러 키로(경고) 기동. 지갑은 번들된 network.json의 identity를 고정 신뢰.
  - 검증: 메시지 30% 유실·재정렬에서도 합의(시드 3개), 이중 로그 딜러 전원 제외, 4프로세스 DKG→합의→identity로 잔액 검증·딜러 identity 거부(통합 테스트). **실망: 이 맥(검증자 1~3)과 다른 회선의 poc-m3(검증자 4)가 인터넷 너머로 DKG를 마치고 같은 identity로 합의 중.**
- 검증자 교체 — **재공유 + 에포크 전환 구현됨** (`aether reshare`, `epochs.rs`):
  - 옛 위원회 share 보유자가 딜러, 새 검증자 집합이 플레이어(Desmedt 재공유). **identity 불변**, share는 새로 뽑힘, 떠나는 검증자의 share는 삭제. 옛 검증자가 일부 오프라인이어도 딜러 로그 정족수면 완료.
  - 새 위원회는 새 에포크로 시작: `network.json.epochs`에 (시작 높이, 직전 블록 해시). `ScheduleEpocher`가 에포크 경계를 정하고 Commonware `Deferred`가 직전 에포크 마지막 블록 위에 이어 짓는다. 투표 저널은 에포크별 파티션(옛 위원회 투표를 새 서명자로 재생하지 않음). `RotatingProvider`가 현재 에포크는 서명 scheme, 지난 에포크는 같은 그룹키 검증 scheme을 준다(새로 합류한 검증자가 옛 인증서를 검증하며 따라잡음).
  - 절차(정지 후 교체): 옛 위원회 정지 → `aether head --data`로 마지막 확정 높이·해시 → 모든 옛·새 검증자가 `aether reshare --from 현재 --to 다음 --epoch-end H --epoch-end-hash X` → 새 위원회가 reshare가 쓴 network.json으로 기동.
  - 안전장치: 노드는 network.json에 identity가 없거나 threshold.json 라운드가 다르면 기동 거부. 에포크 부모 해시가 자기 확정 블록과 다르거나 경계 너머까지 확정했으면 기동 거부. 이미 확정한 높이에 다른 블록이 오면 `CONFLICTING FINALIZED BLOCK` 오류(예전엔 조용히 무시했음). 저장소 파티션 이름을 인덱스와 분리(`<data>/partition`).
  - 검증: 상태기계 재공유(교체·오프라인 딜러), 통합 테스트 `validator_rotation_continues_the_chain_under_the_same_identity` — A={1,2,3,4}가 체인 진행 후 정지, B={2,3,4,5}로 재공유, 빈 데이터로 합류한 5번 포함 B가 **같은 체인**을 이어가고(경계 블록 해시 일치), A 시절 잔액을 5번에서 같은 identity로 검증.
  - 남은 것: 체인 안에서(무정지) 재공유하는 온체인 DKG, VRF 위원회 선출(D9).
- 검증자 키 — **로컬 생성 구현됨**: `aether keygen --data d`가 합의 ed25519 키와 iroh 노드 키를 만들어 `validator.key`(600, 덮어쓰기 거부)에 두고 공개 절반만 `validator.pub.json`으로 낸다. `aether network a.json b.json …`가 `network.json`(체인 id, 검증자 키·노드 id)을 만든다. 노드·DKG는 `--network`로 자기 키를 찾아 인덱스를 정하고, DKG가 identity를 network.json에 더한다. 지갑은 그 network.json 하나로 노드 id(DHT 조회)와 위원회 키를 받는다. 실망은 이 방식으로 재구성: poc-m3의 비밀키는 poc-m3 밖으로 나간 적 없음. `--network` 없이 띄우면 예전처럼 공개 devnet 키.
- 주의(Commonware 문서): 라운드 시드는 같은 라운드 실행에 쓰면 안 된다(리더가 시드를 먼저 알 수 있음). 실행에 난수를 쓸 때는 k라운드 뒤 시드를 약정-공개 방식으로 쓴다.
- 미완: VRF로 에포크마다 고가동 위원회를 뽑는 D9는 위원회가 바뀔 때마다 재공유(DKG)가 필요해 DKG와 함께 구현한다. 지금은 검증자 전원이 위원회.

### (원 설계)

- `consensus/src/engine_simplex.rs`가 Commonware `Automaton`(propose/verify), `Relay`(broadcast), `Committer`(prepared/finalized)를 구현.
- 인증서 = BLS 임계 서명 48바이트. 지갑·재귀 회로가 검증하는 유일한 합의 객체.
- 결정적 런타임(`commonware-runtime` deterministic)으로 시뮬레이션 테스트.

## 위원회 (검증자 전원 ≠ BFT 참여자)

```
후보 풀: 등록된 검증자 (초대 그래프 + Secure Enclave 증명)
에포크 E 시드 = 인증서(E-1 마지막 블록) 해시
위원회(E) = VRF(시드, 후보) 상위 N명, 가중치 = 가동률 평판 × (스테이크, 토큰 도입 후)
N = min(후보 수, COMMITTEE_MAX)   // 초기 7~21, 50 미만이면 Simplex 그대로
```

- 구현(`crates/consensus/src/committee.rs`): ticket = H(시드 ‖ id)의 상위 64비트, 점수 = ticket / 가중치(낮을수록 선출), u128 교차곱으로 비교. **정수 연산만** 사용(부동소수점 ln은 libm마다 달라 합의가 갈린다). 시드는 이전 에포크 확정 인증서 해시.
- 가동률 평판: 최근 K에포크 투표 참여율. 오프라인이면 다음 에포크 선출 확률 하락, 페널티 없음.
- 위원회 밖 검증자: 블록 실행·검증·증명은 하되 투표 안 함. 지갑은 이들의 증명을 그대로 쓴다.

## ebb-and-flow

- 가용 체인: 위원회 2f+1 미달로 인증서가 안 나와도 생산자는 블록을 계속 제안(가용 체인, 확정 없음).
- 최종성 가젯: 위원회가 회복되면 가용 체인의 prefix를 일괄 확정.
- 지갑 표시: "확정됨(인증서)" / "제안됨(미확정)" / "증명됨(ZK)" 세 상태를 구분.

## 포함 목록 (FOCIL식, 1단계) — 구현됨

- 위원회: 검증자 ≤ 8이면 전원, 그 이상이면 높이마다 8명 창이 회전(`inclusion::committee`).
- 멤버는 블록 주기마다 자기 멤풀에서 주기 이상 기다린 가장 오래된 tx ≤ 16개를 **tx 본문째** ed25519로 서명해 gossip(p2p 채널 6). 받은 노드는 서명·멤버 자격·무상태 유효성을 검사하고 (멤버, 높이)당 한 번 재전파한다.
- 생산자: 보유한 목록 tx를 (nonce, 발신자) 순으로 맨 앞에 넣고, 나머지는 멤풀.
- 투표자: 블록 재실행 후 **append check** — 목록 tx가 블록에 없고, 블록 사후 상태에 이어 붙였을 때 유효하며 남은 가스에 들어가면 위반 → 투표 거부(nullify로 다음 리더에게). 블록이 가득(tx 수 상한)이면 면제.
- 동결: 투표자는 받은 지 `FREEZE`(750ms) 지난 목록만 강제한다. 재전파로 그 사이 생산자에게도 도달한다.
- 강제는 **투표 규칙**이지 유효성 규칙이 아니다. 확정 블록은 목록으로 재판정하지 않는다(백필·재실행 결정성 유지).
- 검증: 모든 노드가 특정 발신자를 자기 정렬에서 빼고(`--dev-deprioritize`) 검증자 1은 목록까지 무시(`--dev-censor`)해도, 그 tx가 목록 경로로만 확정된다(`tests/devnet.rs::inclusion_lists_get_censored_txs_in`). 목록 발행을 끄면 같은 테스트가 40초 안에 확정 못 함을 확인(대조 실험).
- 암호 없음. 검열 저항의 첫 단계. 다음은 타임락 암호화 멤풀(2단계).

## 순서 정책

`OrderingPolicy` 구현체(03 참조). 1단계 `FifoWithInclusion`: 포함 목록 먼저, 나머지 수수료순.

## 확장 경로

- 50대 초과: Alpenglow식 20+20 이중 정족수(80% 빠른 경로 1라운드 / 60% 느린 경로), 또는 Minimmit crate 출시 시 교체. `engine_*.rs`만 바뀐다.
- 서명 이행: BLS → 해시 기반 XMSS 집계(leanSig). Certificate에 `scheme` 필드 예약.

## Quint 명세 (`specs/consensus.qnt`)

- 모델: 위원회 선출, Simplex 라운드, 인증서, ebb-and-flow 전환, 포함 목록 규칙.
- 불변식: 안전성(상충 인증서 없음), 최종성 단조, 포함 목록 누락 블록은 확정 불가.
- 모델 기반 테스트: Quint 트레이스 → Rust `consensus` 크레이트에 재생(Malachite 방식).

## 시빌 저항 (테스트넷)

- 등록 = 초대 코드(기존 검증자 서명) + Secure Enclave 증명(DeviceCheck/App Attest 계열, macOS에서는 SE 키 증명).
- 토큰·스테이크 없음. 법률 검토 전까지 유지.
