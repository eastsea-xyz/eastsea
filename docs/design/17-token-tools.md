# 17. 토큰 도구: 잠금과 베스팅 — 아무도 못 당긴다 (2026-09-29)

토큰 잠금·베스팅 컨트랙트(`contracts/src/TokenLocker.sol`)의 설계 문서다. [12-launch-plan.md](12-launch-plan.md)의 원칙("남의 자산을 맡지 않음(비수탁), 수수료 0, 불변·무관리자 컨트랙트")과 런치패드 행의 공정 출시 장치(만든 사람 자기 매수 상한, 출시 직후 대량 매수 제한)를 받아, 런치패드 페이지와 지갑이 **"만든 사람 토큰이 언제까지 잠겨 있는지"**를 온체인에서 보여 주는 장치를 만든다. 벤치마크는 [popular-utilities-2026.md](../research/popular-utilities-2026.md)의 Jupiter Lock(무수수료 공공재 락업)·Streamflow(무신뢰 에스크로+클리프)·Sablier(선형 베스팅) 3종.

> 이 문서는 토큰 도구 전반의 공용 설계 문서다. 잠금·베스팅(이번 작업)이 먼저 쓰고, 머클 에어드랍 과제가 뒤에 같은 파일에 붙인다.

## 한 문장

> 잠긴 토큰은 시간이 풀기 전엔 누구도(만든 사람도) 꺼낼 수 없고, 시간은 늘릴 수만 있으며, 베스팅은 만든 사람이 취소 가능을 선택했을 때만 취소되고 취소해도 이미 베스팅된 몫은 수령자에게 나간다.

## 원칙

| 항목 | 규칙 |
|---|---|
| 관리자 | 없음. 배포 후 어떤 설정도 바꿀 수 없고 업그레이드 경로도 없음 |
| 수수료 | 0 (Jupiter Lock처럼 순수 공공재) |
| 예치 방식 | 만든 사람이 `approve` 후 컨트랙트가 `transferFrom`으로 당겨 옴(에스크로) |
| 수수료 온 트랜스퍼 토큰 | 요청액이 아니라 **실제로 도착한 잔액 변화**를 기록해 그만큼 잠금·베스팅 |
| 재진입 | 모든 지급은 **상태 정산 먼저, 토큰 이동 나중**. 재진입 토큰이 다시 와도 이미 정산돼 있어 실패 |
| 이상한 토큰 | `transfer`/`transferFrom`이 bool을 돌려주지 않는 토큰도 허용(반환값 무시 규칙), 잔액 조회가 32바이트가 아니면 거부 |

## 구조: 컨트랙트 두 개

| 컨트랙트 | 쓸 곳 |
|---|---|
| `TokenLocker` | 팀·LP 토큰을 한 시점까지 잠금. 런치패드 "만든 사람 물량 잠금" 배지 |
| `TokenVesting` | 클리프(선택)가 있는 선형 베스팅. 팀·어드바이저·그랜트 분배 |

두 컨트랙트는 `TokenEscrow`(abstract)에서 입출금 플러밍(잔액 변화 측정, bool 반환값 관대 처리)만 공유하고, 상태는 각자 자기 에스크로에 따로 보관한다.

## 잠금 (TokenLocker)

| 항목 | 규칙 |
|---|---|
| `lock(token, beneficiary, amount, unlockAt)` | 호출자(만든 사람)가 `amount`를 예치하고 `beneficiary`에게 잠금. `unlockAt`은 현재보다 미래여야 하고 `beneficiary`는 0 주소면 안 됨. 실제 도착한 양이 잠긴다(0이면 거부) |
| `withdraw(id)` | **수령자만**, unlockAt 이후에, 잠긴 전액을 한 번에 |
| `extend(id, newUnlockAt)` | 잠금 시각을 **뒤로만** 미룬다(같은 시각도 불가). 만든 사람과 수령자만 호출 가능. 이미 인출한 잠금은 불가 |
| 단축 | 방법 없음. 관리자도 없음 |
| 누가 잠금을 만드나 | 누구나. 남을 수령자로 지정해 잠글 수도 있다(락업 증명용) |

이벤트: `Deposited(id, token, beneficiary, creator, amount, unlockAt)`, `Extended(id, from, to)`, `Withdrawn(id, token, beneficiary, amount)`.

## 베스팅 (TokenVesting)

| 항목 | 규칙 |
|---|---|
| `create(token, beneficiary, amount, start, cliff, duration, cancelable)` | `start`부터 `start+duration`까지 선형. `cliff`은 0 이상 `duration` 이하(클리프 전엔 0원 베스팅, 클리프 순간 그때까지 선형 적립액이 한꺼번에 열림). `duration == 0`이면 거부. `start`는 과거(소급)·미래 모두 가능. **기본은 취소 불가** |
| `vested(id)` | 지금까지 베스팅된 양(뷰). 취소 후엔 취소 시점 entitlement로 고정 |
| `claimable(id)` / `claim(id)` | **수령자만** 언제든, 베스팅액에서 아직 안 받은 만큼 |
| `cancel(id)` | `cancelable == true`로 만든 스트림에서만, **만든 사람만**. 이미 베스팅된 몫은 수령자에게, 나머지(베스팅 안 된 몫)는 만든 사람에게 돌아가고 스트림 종료. 취소로 이미 베스팅된 몫을 회수할 수는 없다 |

이벤트: `StreamCreated(id, token, beneficiary, creator, amount, start, cliffEnd, end, cancelable)`, `Claimed(id, token, beneficiary, amount)`, `Canceled(id, paidToBeneficiary, refundedToCreator)`.

계산: `vested = deposited × (now − start) / duration` (클리프 전 0, `end` 이후 전액). 오버플로는 0.8의 체크 연산이 막는다(현실적 물량에서는 도달 불가).

## 화면을 위한 조회 (런치패드 페이지·지갑)

| 함수 | 쓸 곳 |
|---|---|
| `lockedTotal(beneficiary, token)` | 그 주소의 `token`이 아직 이 에스크로에 잠겨 있는 총량 |
| `lockedUntil(beneficiary, token)` | 그 잠금들 중 가장 늦은 unlockAt — **"만든 사람 토큰이 …까지 잠김"** 배지. 없으면 0 |
| `lockIdsOf(beneficiary)` / `lockAt(id)` | 지갑의 "받기로 한 잠금 목록" 화면 |
| `streamIdsOf(beneficiary)` / `streamAt(id)` / `vested` / `claimable` | 지갑의 베스팅 목록·수령 버튼 |

런치패드 토큰 페이지는 (만든 사람, 토큰)으로 `lockedUntil`/`lockedTotal`을 읽어 배지를 그리고, 지갑은 `lockIdsOf(내 주소)`로 잠금·베스팅 목록을 그린다. 이벤트(`Deposited`/`Extended`/`Withdrawn`/`StreamCreated`/`Claimed`/`Canceled`)가 모든 상태 변화를 남기므로 화면은 이벤트 색인 + 뷰 호출로 구성한다.

## 보안

- **정산 우선**: `withdraw`·`claim`·`cancel`은 토큰을 옮기기 전에 자기 상태를 끝낸다. 재진입(ERC-777류·악의적 토큰)이 같은 지급을 다시 시도하면 `AlreadyWithdrawn`/`NothingToClaim`으로 실패한다. 테스트는 지급 중 콜백으로 재시도하는 토큰·수령자 쌍으로 확인했고, 정산 순서를 일부러 틀게 만든 변형에서 테스트가 실패함을 확인했다.
- **권한 최소**: 시각 연장은 그 잠금의 만든 사람·수령자만, 인출·수령은 수령자만, 취소는 만든 사람만(가능 표시 스트림만).
- **수수료 온 트랜스퍼**: `balanceOf` 변화로 측정. 나갈 때 토큰이 또 수수료를 떼면 수령자가 적게 받는 것은 토큰 정책이고, 에스크로 잔액은 항상 정확하다.

## 테스트 (`contracts/test/TokenLocker.t.sol`, forge 24건)

잠금: 잠금→만기→인출(이벤트·잔액), 만기 전·타인 인출 거부, 이중 인출 거부, 연장(만든 사람/수령자/타인, 단축·동일 시각 거부, 인출 후 연장 거부), 잘못된 잠금 거부(0 수령자·과거 unlockAt), 0 예치 거부, 런치패드 뷰(여러 토큰·여러 잠금 집계, 인출 후 갱신), 수수료 온 트랜스퍼 잠금, 인출 재진입 차단.
베스팅: 선형 중간 수령·만기 전액 수령·재수령 거부, 클리프(직전 0·직후 적립 일괄), 클리프==기간, 기간 0·클리프>기간·0 수령자 거부, 수령자 전용, 미래 start(시작 전 0)·과거 start(소급), 취소(중간: 베스팅분 수령자·나머지 만든 사람, 이후 수령·재취소 거부, 클리프 전: 전액 환급, 만기 후: 전액 수령자), 기본 취소 불가·만든 사람 전용, 수수료 온 트랜스퍼 베스팅·취소, 수령 재진입 차단, 없는 id 거부.

## 남은 것

- 런치패드 화면·지갑 앱에 위 조회 함수를 잇는 일(별도 과제).
- 머클 에어드랍 컨트랙트가 이 파일에 이어 붙는다(별도 과제).
