# 07. 합의 계층

## 엔진: Commonware simplex (BLS 임계 scheme)

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

## 포함 목록 (FOCIL식, 1단계)

- 에포크마다 포함 위원회 8명 별도 선출.
- 각 멤버가 자기 멤풀에서 `inclusion_list`(tx 해시 ≤ 16개) 서명 후 gossip.
- 생산자는 블록에 합집합을 포함해야 하고, 검증자는 (가스 여유가 있는 한) 누락 시 블록 거부.
- 암호 없음. 검열 저항의 첫 단계.

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
