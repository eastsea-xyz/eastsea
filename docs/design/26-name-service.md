# 26. 이름 서비스 (.aeth)

주소 대신 이름. 12-launch-plan.md E18 그대로: **고정 소각 요금, 경매 없음.** 오너·관리자·업그레이드·일시정지가 없는 불변 컨트랙트이고, 요금은 전액 소각됩니다 — 누구도 수수료를 받지 않습니다 (원칙: 비수탁, 수수료 0, 추천·순위 없음). 소유자는 주소이고 Aether 계정은 스마트 계정이므로 컨트랙트가 오너일 수 있습니다. 이 컨트랙트는 오너를 호출하지 않습니다: 소유권은 어떤 콜백 표면도 주지 않습니다.

소스: `contracts/src/EastSeaNames.sol` (`EastSeaNames`), 테스트: `contracts/test/EastSeaNames.t.sol`. Rust 변경 없음.

## 모델

```
이름 등록 (1년, 커밋-리빌)
  commit(keccak256(이름 ‖ 소유자 ‖ 솔트))     ← 이름을 숨긴다
      60초 ~ 24시간 기다린다
  register(이름, 소유자, 솔트) + 요금         → 요금 소각, 초과분 같은 호출에 환급
만료: 만료 시각 + 365일부터 다시
  만료 전·유예 30일 안: 누구나 renew (요금 동일·소각, 만료일부터 +365일)
  유예까지 지나면 해방: 뷰는 즉시 조용해지고, 다음 등록이 옛 기록을 초기화
```

## 규칙

| 규칙 | 값 | 이유 |
|---|---|---|
| 문자 | 소문자 ASCII `[a-z0-9-]` | 대소문자 혼동·동형 문자 차단 |
| 길이 | 3–32 | 1–2자는 사치품, 32자는 충분 |
| 붙임표 | 처음·끝 불가. **3–4번째 글자 연속 불가** | `xn--` 퓨니코드(동형 도메인) 원천 차단. 2–3번째(`a--b`)는 허용 |
| 등록 기간 | 365일 | 갱신으로 이어 붙인다 |
| 유예 | 만료 후 30일 | 놓친 갱신의 구제 창. 그 안엔 갱신만 가능(타인 등록 불가) |
| 갱신 | 누구나 (`renew`) | 오너에 대한 선물. 요금은 등록과 동일 |
| 이전 | 2단계: `transferPropose` → `transferAccept` | 잘못 넣은 주소를 승인 전에 회수 가능. `propose(0)`은 취소 |
| 주소 기록 | 1개 (`setAddr`) | 이름 → 주소 |
| 텍스트 기록 | 키 최대 4개, 키 ≤32바이트 `[a-z0-9-]`, 값 ≤128바이트 | 가스·스팸 상한. 빈 값은 삭제(슬롯 회수) |
| 역방향 기록 | 주소 → 이름, 정방향이 그 주소를 가리킬 때만 | 남의 주소에 이름을 못 붙임 |

## 요금 (전액 소각)

| 길이 | 요금 |
|---|---|
| 3자 | 2 AETH |
| 4자 | 0.5 AETH |
| 5자 이상 | 0.1 AETH |

짧은 이름은 희소하므로 비쌉니다 — 가격은 경매가 아니라 길이로 정해집니다. 예약·프리미엄·허가 목록 없음: 커밋-리빌 순서를 지킨 첫 사람이 이깁니다.

- **소각 주소**: `0x0000…dEaD` (생태계 관례의 키리스·코드리스 주소). `address(0)`은 쓰지 않는다 — 이 컨트랙트 전체에서 "없음" 표식이기 때문.
- **환급**: 초과 지불은 같은 호출에서 호출자에게 돌아갑니다. 실패하면 전체가 되돌아갑니다(되돌림 금액도 소각되지 않음).
- **재진입**: 환급(`call`)은 모든 상태 변경 뒤에 옵니다(체크-이펙트-인터랙션). 환급 수신 중 재진입한 악성 컨트랙트는 커밋먼트가 이미 지워져 `UnknownCommitment`를 만나고, 별도 커밋을 준비한 경우엔 정상 등록으로 처리됩니다(요금 정확히 소각) — 둘 다 테스트로 확인.

## 선점 방지: 커밋-리빌

등록 트랜잭션이 공개되는 순간 이름이 노출되므로, 리빌 전에 `commit(해시)`로 이름을 숨깁니다.

- 해시는 `keccak256(이름 ‖ 소유자 ‖ 솔트)` — **소유자가 해시에 묶입니다.**
- 커밋먼트는 60초(`MIN_COMMIT_AGE`) 뒤에야 리빌할 수 있고 24시간(`MAX_COMMIT_AGE`) 안에 해야 합니다. 같은 해시 재커밋으로 창을 다시 열 수 있습니다.
- 공격자가 피해자의 리빌을 관찰해 복사해도: 피해자의 솔트는 공격자 커밋먼트를 열지 못하고(`UnknownCommitment`), 지금 새 커밋을 만들면 60초를 못 기다린 사이 피해자의 이미 숙성된 리빌이 먼저 들어갑니다(`CommitTooNew`).
- 남는 위험은 리빌 트랜잭션 자체가 막히는 것뿐 — 더 높은 가스로 재전송하면 되고, 24시간이 지락하면 솔트를 바꿔 다시 커밋하면 됩니다. ENS와 같은 트레이드오프입니다.
- 60초인 이유: 같은 블록·직후 블록에서의 "커밋 즉시 리빌"을 막는 최소한의 시간. 길게 잡으면 사용성만 나빠집니다.

## 만료와 해방

만료 시각 `e` 이후 30일(`GRACE_PERIOD`) 동안 오너는 모든 권리를 유지하고 갱신할 수 있습니다. `e + 30일`부터 이름은 해방:

- 뷰(`ownerOf`·`addrOf`·`textOf`·`pendingOwnerOf`·`reverseOf`)는 즉시 0/빈값을 돌려줍니다.
- 해방된 자리의 재등록은 옛 기록(주소·텍스트·역방향 클레임)을 전부 초기화합니다 — 해방된 이름은 낡은 리졸버 데이터를 남기지 않습니다.
- 갱신은 현재 만료일부터 +365일입니다(지금부터가 아니라). 유예 기간에 갱신하면 흘러간 시간은 오너의 손해입니다.
- `expiresOf`는 원시 만료일을 그대로 돌려주므로 앱이 유예 종료 시각(`e + 30일`)을 계산할 수 있습니다.

## 지갑이 이름을 읽는 법

| 방향 | 호출 | 뜻 |
|---|---|---|
| 정방향 (이름 → 주소) | `nodeFor(이름)` → `addrOf(node)` | 송금 주소. 해방된 이름은 0 |
| 역방향 (주소 → 이름) | `reverseOf(주소)` | 주소의 대표 이름 |
| 상호 확인 | `addrOf` 결과의 `reverseOf`가 같은 이름을 돌려주는지 | 이름이 그 주소의 것이 맞는지 확인 — UI는 통과할 때만 이름을 강조 |

역방향은 컨트랙트가 읽을 때마다 재검증합니다(살아있는 기록 + 정방향이 되돌아보기). 그래서 만료·이전·주소 변경 뒤의 낡은 클레임은 자동으로 조용해지고, `setAddr`은 옛 주소의 클레임을 즉시 회수합니다. 앱은 아무것도 믿고 재검증할 필요가 없습니다.

## 이벤트

| 이벤트 | 뜻 |
|---|---|
| `CommitmentMade(해시)` | 커밋 등장 (이름은 숨겨짐) |
| `Registered(이름, node, 오너, 만료, 요금)` | 등록 완료 |
| `Renewed(node, 새만료, 요금)` | 1년 연장 |
| `Burned(금액)` | 요금 소각 (`Registered`/`Renewed` 직후) |
| `TransferProposed/Accepted(node, from, to)` | 2단계 이전 |
| `AddrSet(node, 주소)` | 주소 기록 변경 |
| `TextSet(node, 키, 값)` | 텍스트 기록 설정/삭제 |
| `ReverseSet(계정, node, 이름)` | 역방향 클레임 |

## 테스트 (`cd contracts && forge test`)

| 테스트 | 확인 |
|---|---|
| `test_NameGrammarRules` | 22개 표본(경계·`xn--`·대문자·33자·빈 문자열)과 독립 문법 참조의 교차 검증 |
| `testFuzz_NameValidationMatchesSpec` | 문법 밖은 절대 통과, 안은 절대 거부 (5000 런) |
| `test_FeeBuckets` / `test_RegisterBurnsExactFeeAndRefundsOverpayment` / `test_RegisterUnderpayReverts` | 길이별 고정 요금, 정확히 소각·초과분 환급, 미달 `InsufficientFee` |
| `test_RegisterRequiresMatchingCommitment` / `test_CommitAgeWindow` / `test_CommitmentSpentOnRegister` | 커밋먼트 일치·소유자 바인딩, 60초/24시간 경계, 리빌 후 소진 |
| `test_FrontRunCannotStealPendingRegistration` | 리빌 복사·즉시 커밋-리빌 모두 실패 |
| `test_RegisterHappyPath` / `test_RegisterForAnotherAddress` / `test_RegisterTakenName` | 등록 이벤트·만료, 대납(지불자≠소유자), 중복 `NameTaken` |
| `test_ExpiryBoundaries` / `test_ReleasedNameRegistersFreshAndKeepsNoStaleData` | 만료 1초 전/직후/유예 마지막 초/해방 직후, 재등록 시 낡은 데이터 전멸 |
| `test_RenewalByAnyoneExtendsFromExpiry` / `test_RenewUnknownName` | 제3자 갱신·만료일 기준 연장, 미등록 `Unregistered` |
| `test_TransferTwoStep` / `test_TransferGuards` | 제안→승인, 오너 아닌 제안·승인, 취소, 덮어쓰기 |
| `test_SetAddrOwnerOnly` | 주소 기록 오너 전용 |
| `test_TextRecordBounds` / `test_TextRecordValidation` | 4키 상한·슬롯 회수·no-op 삭제, 키/값 크기·문자집합 |
| `test_ReverseRequiresPointback` / `test_ReverseCannotClaimSomeoneElsAddress` | 포인트백 없으면 거부, 남의 주소에 클레임 불가 |
| `test_ReverseSurvivesTransferUntilForwardMoves` / `test_ReverseOfUnclaimedAddress` | 이전 후에도 정방향이 유지되는 동안 클레임 유효, 미클레임 주소 |
| `test_ReentrantRefundCannotDoubleRegister` / `test_ReentrantRefundLegitimateSecondRegister` | 환급 중 재진입: 이중 등록 실패 / 별도 커밋의 정상 등록은 소각 총액 정확 |
| `test_MaliciousOwnerContractIsInert` | 컨트랙트 오너 전 사이클 — 오너 콜백 0회 |
| `testFuzz_StateInvariants` | 무작위 연산 흐름 뒤: 소각 총액 = 수수료 합 = 소각 주소 잔액, 이름마다 오너 정확히 하나(모델 일치), 모든 유효 역방향은 정방향과 정합 (5000 런) |

테스트는 forge-std 없이 자체 `Vm` 인터페이스로 돕니다 (저장소의 forge 0.2.0). `FOUNDRY_FUZZ_RUNS=5000`으로 두 퍼즈 테스트가 각 5000 런을 돕니다.

## 의도적으로 만들지 않은 것

- **경매·가격 탐색** — 요금은 길이로 고정. 입찰·시세·종가 없음.
- **예약·프리미엄·허가 목록** — 특정 이름을 특별 취급하지 않음.
- **마켓플레이스·임대·담보** — 이름 매매는 소유자 간 2단계 이전으로 직접.
- **서브도메인** — `a.b` 형태 확장 없음. 레코드는 최상위 이름 하나에만.
- **수수료 수신자·수익 배분** — 받는 사람 자체가 없음. 전액 소각.
- **관리자 구제** — 실수로 잃은 이름의 복구 경로 없음. 유예 30일이 마지막 기회.

## 남은 것 (나중)

- 지갑 앱 이름 화면(검색·커밋-리빌 등록·만료 알림·상호 확인 표시) — 이 문서의 이벤트/뷰 규격을 따름.
- 배포는 하지 않았다 (메인넷 전 승인 후).
