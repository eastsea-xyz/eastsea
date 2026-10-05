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
| `lockedTotal(beneficiary, token)` | 그 주소의 `token`이 아직 이 에스크로에 잠겨 있는 총량 (전체 이력 스캔 — 아래 F-04 경고) |
| `lockedUntil(beneficiary, token)` | 그 잠금들 중 가장 늦은 unlockAt — **"만든 사람 토큰이 …까지 잠김"** 배지. 없으면 0 (전체 이력 스캔 — 아래 F-04 경고) |
| `lockCountOf(beneficiary)` + `lockIdsOfPage(beneficiary, offset, limit)` | 지갑의 "받기로 한 잠금 목록" 화면. 개수를 먼저 읽고 묶음(페이지)으로 걷는다 |
| `streamCountOf(beneficiary)` + `streamIdsOfPage(beneficiary, offset, limit)` + `streamAt(id)` / `vested` / `claimable` | 지갑의 베스팅 목록·수령 버튼, 같은 방식으로 페이지 단위 |
| `lockIdsOf(beneficiary)` / `streamIdsOf(beneficiary)` | **새 소비자는 쓰지 않는다(F-04, 폐기 예정)**. 배열 전체를 한 번에 돌려주므로 이력이 커지면 가스·지연 한계를 넘는다. 기존 ABI 호환용으로만 남아 있다 |

**F-04(감사 2026-10-05, 중간 등급)**: 누구나 수령자 하나를 골라 무한한 dust 잠금을 쌓을 수 있고, 인출된 잠금도 `_idsOf`에 남는다. `lockedTotal`/`lockedUntil`은 항상 이력 전체를 훑으므로 카디널리티가 커지면 이 호출 하나가 가스·지연 예산을 넘을 수 있다 — 유효한 잠금이 있어도 배지가 죽는다. 그래서 (1) 온체인 소비자는 `lockCountOf` + `lockIdsOfPage`(묶음별 조회)로 유계하게 걷고, (2) 배지는 색인기가 페이지를 모아 계산·캐시하며, (3) 보안 판단을 이 단일 무한 스캔에 걸지 않는다. 회귀 테스트는 dust 300개에 파묻힌 실제 잠금을 페이지 순회로 복원한다.

런치패드 토큰 페이지는 (만든 사람, 토큰)으로 `lockedUntil`/`lockedTotal`을 읽어 배지를 그리고, 지갑은 페이지 조회로 잠금·베스팅 목록을 그린다. 이벤트(`Deposited`/`Extended`/`Withdrawn`/`StreamCreated`/`Claimed`/`Canceled`)가 모든 상태 변화를 남기므로 화면은 이벤트 색인 + 뷰 호출로 구성한다.

## 보안

- **정산 우선**: `withdraw`·`claim`·`cancel`은 토큰을 옮기기 전에 자기 상태를 끝낸다. 재진입(ERC-777류·악의적 토큰)이 같은 지급을 다시 시도하면 `AlreadyWithdrawn`/`NothingToClaim`으로 실패한다.
- **예치 재진입 가드(F-01, 감사 2026-10-05, 높음)**: 정산 순서만으로는 부족했다 — 콜백 가능 토큰이 `_pull` 도중에 재진입해 중첩 예치를 만들면, 중첩 기록과 바깥 호출의 전체-델타 크레딧이 같은 토큰을 이중으로 센다(감사 PoC에서 각 에스크로가 200을 보유한 상태로 300을 약속). 두 에스크로의 모든 상태 변경 진입점이 `nonReentrant` 가드 아래에서 돌고, 예치 중 지급 시도(델타를 깎아 바깥 예치를 왜곡하는 변종)도 막는다. 자금 보존 퍼즈(정상 토큰 정등식, 공격 토큰 `balance ≥ 미지급 총액`)가 이를 고정한다.
- **권한 최소**: 시각 연장은 그 잠금의 만든 사람·수령자만, 인출·수령은 수령자만, 취소는 만든 사람만(가능 표시 스트림만).
- **수수료 온 트랜스퍼**: `balanceOf` 변화로 측정. 나갈 때 토큰이 또 수수료를 떼면 수령자가 적게 받는 것은 토큰 정책이고, 에스크로 잔액은 항상 정확하다.

## 테스트 (`contracts/test/TokenLocker.t.sol`, forge 27건; `contracts/test/TokenEscrowReentrancy.t.sol`, 8건)

잠금: 잠금→만기→인출(이벤트·잔액), 만기 전·타인 인출 거부, 이중 인출 거부, 연장(만든 사람/수령자/타인, 단축·동일 시각 거부, 인출 후 연장 거부), 잘못된 잠금 거부(0 수령자·과거 unlockAt), 0 예치 거부, 런치패드 뷰(여러 토큰·여러 잠금 집계, 인출 후 갱신), 수수료 온 트랜스퍼 잠금, 인출 재진입 차단, 페이지 조회(중간 묶음·끝 넘김·빈 결과·모르는 수령자)와 dust 300개가 묻은 이력의 페이지 복원(F-04).
`TokenEscrowReentrancy.t.sol`(F-01): 중첩 lock/create 차단(이중 크레딧 없음), 예치 중 withdraw/claim 차단(델타 왜곡 없음), 되돌리는 콜백이 예치 전체를 롤백, 그리고 정상 토큰(정등식)·공격 토큰(`balance ≥ 미지급`) 자금 보존 퍼즈.
베스팅: 선형 중간 수령·만기 전액 수령·재수령 거부, 클리프(직전 0·직후 적립 일괄), 클리프==기간, 기간 0·클리프>기간·0 수령자 거부, 수령자 전용, 미래 start(시작 전 0)·과거 start(소급), 취소(중간: 베스팅분 수령자·나머지 만든 사람, 이후 수령·재취소 거부, 클리프 전: 전액 환급, 만기 후: 전액 수령자), 기본 취소 불가·만든 사람 전용, 수수료 온 트랜스퍼 베스팅·취소, 수령 재진입 차단, 없는 id 거부.

## 남은 것

- 런치패드 화면·지갑 앱에 위 조회 함수를 잇는 일(별도 과제).
- 머클 에어드랍 컨트랙트가 이 파일에 이어 붙는다(별도 과제).

---

## 17-2. 토큰 배포 도구: 머클 청구 캠페인과 일괄 전송 (2026-09-29)

에어드랍과 다중 송금을 위한 도구 모음. 컨트랙트는 [contracts/src/MerkleDistributor.sol](../../contracts/src/MerkleDistributor.sol) 한 파일에 세 개가 들어 있고, 트리와 증명은 [scripts/merkle-build.mjs](../../scripts/merkle-build.mjs)가 만든다.

### 한 문장

> 받을 사람 목록을 CSV로 주면 스크립트가 머클 루트와 증명 JSON을 만들고, 팩토리가 그 루트로 완전히 입금된 청구 컨트랙트를 하나 뽑는다. 청구는 누구나 대신 보낼 수 있고 수수료는 없다.

### 구성

| 이름 | 하는 일 |
|---|---|
| `MerkleDistributorFactory` | 캠페인마다 `MerkleDistributor`를 하나 배포하며 같은 트랜잭션에서 전액 입금한다. CREATE2라 주소가 결정적이다(생성자별 연번 솔트) |
| `MerkleDistributor` | 한 캠페인. 생성 후에는 아무도(생성자도) 토큰을 잡을 수 없고, 트리에 적힌 사람만 받는다 |
| `TokenBatch` | Disperse 방식 ERC-20 일괄 전송. 한 번 승인(approve)하면 각 수령인에게 곧장 `transferFrom`으로 보낸다(예치 없음) |
| `scripts/merkle-build.mjs` | `address,amount` CSV → 정렬된 트리 → 루트·총액·증명 JSON. 의존성 없음(keccak 직접 구현) |

네이티브 AETH 일괄 송금은 이미 계정 자체가 한다(`EastSeaAccount.execute`의 여러 호출). 이 도구는 ERC-20용이다.

### 머클 규격

컨트랙트와 CLI가 같은 규칙을 쓴다. 어느 쪽을 믿어도 같은 루트가 나온다.

| 항목 | 규칙 |
|---|---|
| 리프 | `keccak256(index, account, amount)` (`abi.encodePacked`, 인덱스 포함) |
| 내부 노드 | 두 해시를 **작은 해시가 앞으로** 정렬해 붙여 해시 |
| 홀수 레벨 | 마지막 노드를 자기 짝으로 복제해 쓴다 |
| 인덱스 배정 | CSV 행을 (주소, 금액) 오름차순 정렬한 뒤 0부터 매긴다. 행 순서가 달라도 같은 루트 |
| 증명 | 리프에서 루트까지의 형제 해시 나열. 컨트랙트는 좌우를 다시 정렬해 계산하므로 방향을 몰라도 된다 |

- 리프에 인덱스가 들어가므로 같은 주소가 두 번 listed 되어도(보조금 두 건 등) 각각 따로 청구된다.
- 짝 정렬(small-first) 방식이라 트리 빌더와 검증자의 좌우 해석이 어긋날 여지가 없다.

### 캠페인 생애주기

| 단계 | 누가 | 무슨 일 |
|---|---|---|
| 생성 | 생성자(자금 낸 사람) | 토큰으로 팩토리에 승인해 두고 `create(token, root, ends, total)`. `total`이 **정확히** 새 캠페인에 도착해야 성립한다(아래 F-02/F-03). `ends`는 종료 시각(0 = 종료 없음), 과거여서는 안 된다 |
| 청구 | **누구나** | `claim(index, account, amount, proof)`. 증명이 맞으면 토큰은 `account`에게, 가스를 낸 사람에게가 아니라. 인덱스당 한 번 |
| 종료 | — | `ends`가 지나면 청구는 멈춘다. 마지막 순간(ends 1초 전)까지는 청구된다 |
| 회수 | 생성자만 | 종료 후 `sweep()`으로 미청구 잔액을 생성자 주소로. `ends`가 0인 캠페인은 영원히 청구 가능하고 회수도 없다 |

- 설계: **immutable, 무관리자, 무수수료.** 생성자가 남겨 둔 권리는 "종료 후 회수" 하나뿐이고, 그마저 종료 시각은 캠페인 시작 때 자기가 정했다.
- 청구를 중개인(relayer)이 대신 보낼 수 있는 것은 의도된 기능이다. 받는 사람은 서명조차 필요 없다 — 누군가 (index, account, amount, proof)를 제출하면 된다. 가스 스폰서싱·지갑 없는 수령인 모두 지원.
- **입금은 정확해야 한다(F-02/F-03, 감사 2026-10-05)**: 팩토리는 토큰 주소에 코드가 있는지 보고, 입금 전후의 자식 잔액 차가 `total`과 **정확히 같은지** 확인한다. 수수료 온 트랜스퍼 토큰(덜 도착), 초과 지급 토큰(더 도착), 코드 없는 주소(EOA — 빈 반환도 성공으로 읽히던 경로), 거짓 성공 토큰(아무것도 안 옮기고 `true`)은 모두 `NotAContract`/`Underfunded`로 거부되고 캠페인 목록에도 남지 않는다. 특히 `ends == 0`(영구) 캠페인은 회수 경로가 없으므로 덜 도착한 채 살아남으면 토큰이 영구히 갇힌다 — 이 정책의 목적이다. 캠페인 자금은 표준 동작 토큰으로.
- 전송 실패 시 토큰의 원래 revert 사유가 그대로 올라온다.

### TokenBatch (일괄 전송)

```
token.approve(TokenBatch, 총액)
TokenBatch.send(token, [주소들], [금액들])
```

- 각 수령인에게 `transferFrom(송금자 → 수령인)`이 곧장 실행된다. 컨트랙트가 토큰을 한 순간도 들고 있지 않다(예치형이 아니다).
- 전부 아니면 전무: 도중 하나라도 실패하면(승인이 중간에 바닥나는 경우 등) 전체가 되돌아간다.
- 배열 길이가 다르면 `LengthMismatch`.
- **보낸 것만 알린다(F-03)**: 토큰 주소에 코드가 있어야 하고(`NotAContract`), 루프가 끝난 뒤 송금자 잔액이 정확히 `total`만큼 줄었는지 확인한다(`NotMoved`). `Sent` 이벤트는 영수증일 뿐 이체 증명이 아니므로, 거짓 성공 토큰·EOA 토큰이 성공 이벤트를 사는 경로를 막는다. 수수료 온 트랜스퍼 토큰은 여전히 쓸 수 있다 — 송금자가 전액을 내고 도착분이 깎이는 건 수령인과 토큰의 정책이다.

### CLI

```
# recipients.csv: address,amount (양은 최소 단위 정수, # 주석과 헤더 줄 허용)
node scripts/merkle-build.mjs recipients.csv --token 0xTOKEN --out merkle.json
```

출력(stdout; `--out`이면 파일)은 지갑·중개인이 읽는 JSON이다. 모든 청구를 한 줄에 하나씩 쓴다:

```json
{
  "root": "0x7408…",
  "total": "44251000000000000000000",
  "count": 5,
  "claims": [
    {"index":0,"address":"0x…01","amount":"42000000000000000000000","proof":["0x…","0x…","0x…"]}
  ]
}
```

- 스크립트는 출력 전에 모든 증명을 루트에 대해 다시 검증한다(자체 점검).
- 금액은 문자열로 쓴다(JSON 수치는 정밀도 손실 위험).
- 트리·증명 생성기에 keccak256을 직접 구현했다(의존성 없음). 벡터는 `cast keccak`과 대조했고, 아래 연동 테스트가 체인 검증과 맞춰 본다.

### 테스트

```
cd contracts && forge test                      # 23개: 유효/무효 증명, 이중 청구, 종료 전후 회수,
                                                #        승인 부족 일괄 전송의 원자성, 팩토리 입금,
                                                #        F-02/F-03(수수료·EOA·거짓·초과 토큰 입금 거부) 등
node --test scripts/merkle-build.test.mjs       # 6개: keccak 벡터, CSV 검증, 정렬·결정성
```

- **연동(fixture) 테스트**: `contracts/test/fixtures/recipients.csv`(순서 섞임, 같은 주소 두 번 포함)를 CLI로 굽고, forge 테스트가 그 JSON(`merkle.json`)을 읽어 컨트랙트를 배포한 뒤 모든 청구를 온체인에서 검증한다. 솔리디티 쪽에서 트리를 다시 만들지 않는다 — CLI의 출력만으로 루트가 맞고 캠페인이 정확히 바닥나는지 본다.
- 이 저장소의 forge(0.2.0, 2023)는 JSON 치트코드가 없어서, 테스트가 JSON을 파일로 읽어 직접 파싱한다.
- fixture 갱신: CSV를 고치고 재생성 — `node scripts/merkle-build.mjs contracts/test/fixtures/recipients.csv --out contracts/test/fixtures/merkle.json` (CSV 머리에도 적어 두었다).
