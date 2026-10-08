# 26. 이름 서비스 (.aeth)

주소 대신 이름. 12-launch-plan.md E18 그대로: **고정 소각 요금, 경매 없음.** 오너·관리자·업그레이드·일시정지가 없는 불변 컨트랙트이고, 요금은 전액 소각됩니다 — 누구도 수수료를 받지 않습니다 (원칙: 비수탁, 수수료 0, 추천·순위 없음). 소유자는 주소이고 Aether 계정은 스마트 계정이므로 컨트랙트가 오너일 수 있습니다. 이 컨트랙트는 오너를 호출하지 않습니다: 소유권은 어떤 콜백 표면도 주지 않습니다.

소스: `contracts/src/EastSeaNames.sol` (`EastSeaNames`), 테스트: `contracts/test/EastSeaNames.t.sol`. Rust 변경 없음.

**검색 추가 (2026-10-08, 0.7.4):** [앱·이름 중립 검색](app-search.md)은 이 이벤트 ABI로 노드 인덱스를 재구축하고 `.sea` 이름·텍스트를 읽기 전용 RPC로 제공한다. 아래의 “추천·순위 없음”은 이름 컨트랙트의 요금·등록 규칙이며, 검색 정렬은 새 문서의 공개 규칙을 따른다. 검색 리더는 호환 `Registered` 이벤트의 점으로 나뉜 하위 이름도 읽을 수 있지만, 이 컨트랙트에는 하위 이름 등록 기능이 없다. 이 추가는 이름 등록·배포 기능을 변경하지 않는다.

## 모델

```
이름 등록 (1년, 커밋-리빌)
  commit(keccak256(이름 ‖ 소유자 ‖ 솔트 ‖ 릴레이어)) + 본드 0.01   ← 이름을 숨긴다; 본드 즉시 소각
      60초 ~ 24시간 기다린다
  register(이름, 소유자, 솔트, 릴레이어) + (요금 − 본드)
      커미터 직접 리빌: 합계 정확히 요금. 릴레이어 리빌: 전액 요금 (본드는 커미터 몫)
      초과분 같은 호출에 환급
  24시간 지난 미리빌 커밋먼트: clear(해시) — 누구나 슬롯 회수 (본드는 이미 소각됨)
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
| 커밋먼트 슬롯 | 해시당 `(커미터, 시각)` 1슬롯. **살아있는(24시간 미만) 해시의 재커밋은 누구든 `CommitmentActive` revert** — 커미터 본인도 시각 이동 불가 | 커밋먼트를 커미터에 묶어 타임스탬프 리셋 공격 차단 (A5-3) |
| 리빌 권한 | 커미터 또는 해시에 지정된 릴레이어 1명만 (`address(0)` = 없음) | 타인의 커밋먼트 리빌·가로채기 차단 (A5-3). 소유자는 해시 안에 있으므로 대리 등록 유지 |
| 커밋 본드 | `COMMIT_BOND` = 0.01 DBLN, 커밋 시 즉시 소각·불환급. 커미터 직접 리빌 시 요금에서 차감 | 무료 영구 상태 쓰기 차단 (A5-1) |
| 만료 커밋먼트 회수 | 24시간 후 누구나 `clear(해시)` — 슬롯 삭제, 지급 없음 | 방치 슬롯의 스토리지 회수 (A5-1) |

## 요금 (전액 소각)

| 길이 | 요금 |
|---|---|
| 3자 | 2 DBLN |
| 4자 | 0.5 DBLN |
| 5자 이상 | 0.1 DBLN |

짧은 이름은 희소하므로 비쌉니다 — 가격은 경매가 아니라 길이로 정해집니다. 예약·프리미엄·허가 목록 없음: 커밋-리빌 순서를 지킨 첫 사람이 이깁니다.

- **소각 주소**: `0x0000…dEaD` (생태계 관례의 키리스·코드리스 주소). `address(0)`은 쓰지 않는다 — 이 컨트랙트 전체에서 "없음" 표식이기 때문.
- **환급**: 초과 지불은 같은 호출에서 호출자에게 돌아갑니다. 실패하면 전체가 되돌아갑니다(되돌림 금액도 소각되지 않음).
- **재진입**: 환급(`call`)은 모든 상태 변경 뒤에 옵니다(체크-이펙트-인터랙션). 환급 수신 중 재진입한 악성 컨트랙트는 커밋먼트가 이미 지워져 `UnknownCommitment`를 만나고, 별도 커밋을 준비한 경우엔 정상 등록으로 처리됩니다(요금 정확히 소각) — 둘 다 테스트로 확인.

### 커밋 본드 (`COMMIT_BOND` = 0.01 DBLN, 소각)

커밋 한 번은 스토리지 1슬롯(해시당 `(커미터, 시각)`)을 영구히 씁니다. 본드가 없으면 `commit`은 무료 상태 쓰기 통로가 됩니다(감사 5차 A5-1: 커밋당 40,555 gas로 하루 829 MB 상태 성장 가능). 그래서:

- **값**: 최저 등록 요금(5자 이상 0.1 DBLN)의 **1/10 = 0.01 DBLN**. 커밋은 등록의 예비 단계이므로 등록보다 확실히 싸야 하고, 1/10이면 스팸의 단가가 등록 요금 수준까지 올라가 무료 경로가 사라집니다. `COMMIT_BOND × 10 == FEE_5_PLUS`는 테스트로 고정.
- **소각·불환급**: 커밋 시 즉시 `BURN_ADDRESS`로. 예치(escrow)가 아니므로 환급 표면·드레인 위험이 없습니다. 잔액은 정확히 `msg.value == COMMIT_BOND`일 때만 받습니다(초과분 환급 경로조차 스팸 민감 지점에 두지 않음).
- **정직한 사용자의 추가 부담 0**: 같은 커미터가 창 안에서 직접 리빌하면 등록 시 `요금 − COMMIT_BOND`만 내면 됩니다 — 합계가 정확히 등록 요금. 릴레이어가 대신 리빌하면 릴레이어가 전액 요금을 내고 본드는 커미터가 부담(누가 돈을 내는지가 흩어질 뿐 총 소각은 보존).
- **방어 계층 구분**: 임의의 스토리지-라이팅 컨트랙트에 대한 **진짜 방어는 체인 수준 state fee**(블록이 타깃 절반 아래로 내려가 실행·증명 base fee가 0이어도 영구 상태 증가에 과금 — 별도 엔지니어가 `fees.rs`/`block.rs`에서 수정). 이 본드는 어디까지나 *이 컨트랙트가* 무료 경로가 되지 않게 하는 계약 수준 조치입니다. 본드만으로 임의 컨트랙트 스팸은 막지 못합니다.
- **만료 커밋먼트 회수**: 24시간 창이 지난 미리빌 해시는 누구나 `clear(해시)`로 슬롯을 지울 수 있습니다. 지급은 없습니다 — 바운티를 주려면 본드를 보관(환급 가능)해야 하는데, 그 순간 드레인 표면이 생깁니다. clear는 가스 한 번의 대가로 스토리지를 돌려주는 시민의무입니다. clear 후 같은 해시의 새 커밋은 새 커미터의 새 창입니다.

## 선점 방지: 커밋-리빌

등록 트랜잭션이 공개되는 순간 이름이 노출되므로, 리빌 전에 `commit(해시)`로 이름을 숨깁니다.

- 해시는 `keccak256(이름 ‖ 소유자 ‖ 솔트 ‖ 릴레이어)` — **소유자와 (쓰고 싶다면) 릴레이어가 해시에 묶입니다.** 릴레이어는 커밋먼트에 지정된, 커미터 외에 리빌할 수 있는 단 한 계정(예: 지갑 앱의 대낭 주소). `address(0)`이면 커미터만.
- 커밋먼트는 60초(`MIN_COMMIT_AGE`) 뒤에야 리빌할 수 있고 24시간(`MAX_COMMIT_AGE`) 안에 해야 합니다. **살아있는 해시의 재커밋은 누구든 (`CommitmentActive`) revert** — 커미터 본인도 자기 커밋먼트의 시각을 못 옮깁니다. 24시간이 지난 죽은 슬롯만 재커밋으로 교체할 수 있고, 그때부터는 새 커미터의 새 창입니다(옛 커미터는 `NotCommitter`).
- **리빌은 커미터 또는 해시 속 릴레이어만.** 공격자가 피해자의 리빌을 관찰해 파라미터를 복사하고 커밋먼트 해시를 재계산해 더 높은 우선순위로 `commit(C)`를 넣어도 (1) 살아있는 슬롯은 동결돼 revert(시각·커미터 불변 — 감사 5차 A5-3의 타임스탬프 리셋 공격 차단), (2) 피해자의 이미 숙성된 리빌이 그대로 통과합니다.
- 공격자가 피해자의 솔트로 제 이름을 등록하려 해도: 해시에 소유자가 묶여 있어 공격자 계정의 커밋먼트를 열지 못합니다(`UnknownCommitment`), 지금 새 커밋을 만들면 60초를 못 기다린 사이 피해자의 숙성된 리빌이 먼저 들어갑니다(`CommitTooNew`).
- 남는 위험은 리빌 트랜잭션 자체가 막히는 것뿐 — 더 높은 가스로 재전송하면 되고, 24시간이 지나면 솔트를 바꿔 다시 커밋하면 됩니다. ENS와 같은 트레이드오프입니다.
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
| `CommitmentMade(해시, 커미터)` | 커밋 등장 (이름은 숨겨짐, 커미터가 슬롯 주인) |
| `CommitmentCleared(해시, 호출자)` | 만료 커밋먼트 슬롯 회수 |
| `Registered(이름, node, 오너, 만료, 요금)` | 등록 완료 |
| `Renewed(node, 새만료, 요금)` | 1년 연장 |
| `Burned(금액)` | 요금·본드 소각 (`CommitmentMade`/`Registered`/`Renewed` 직후) |
| `TransferProposed/Accepted(node, from, to)` | 2단계 이전 |
| `AddrSet(node, 주소)` | 주소 기록 변경 |
| `TextSet(node, 키, 값)` | 텍스트 기록 설정/삭제 |
| `ReverseSet(계정, node, 이름)` | 역방향 클레임 |

## 테스트 (`cd contracts && forge test`)

| 테스트 | 확인 |
|---|---|
| `test_NameGrammarRules` | 22개 표본(경계·`xn--`·대문자·33자·빈 문자열)과 독립 문법 참조의 교차 검증 |
| `testFuzz_NameValidationMatchesSpec` | 문법 밖은 절대 통과, 안은 절대 거부 (5000 런) |
| `test_FeeBuckets` / `test_RegisterBurnsExactFeeAndRefundsOverpayment` / `test_RegisterUnderpayReverts` | 길이별 고정 요금, 정확히 소각·초과분 환급(본드+잔금=요금), 미달 `InsufficientFee` |
| `test_A5_1_CommitBurnsBondImmediately` / `test_A5_1_CommitRequiresExactBond` | 본드 즉시 소각·컨트랙트 잔액 0, 정확한 값만 허용(0도 초과도 `WrongBondValue`, 슬롯 미기록) |
| `test_A5_1_BondCreditedOnCommitterReveal` / `test_A5_1_TotalBurnedEqualsFeesPlusUnrefundedBonds` / `test_CommitBondRatioPinned` | 커미터 직접 리빌 = 총지출 정확히 요금, `totalBurned` = 수수료 + 미공제 본드(릴레이어·방치·clear 혼합 시나리오), `COMMIT_BOND × 10 == FEE_5_PLUS` 고정 |
| `test_A5_1_ClearGuardsAndEffects` / `test_A5_1_ClearUnknownCommitment` | clear는 만료 후에만(경계 1초 포함)·누구나·이벤트, 슬롯 해제 후 재커밋 가능, 이중 clear·미존재 revert |
| `test_RegisterRequiresMatchingCommitment` / `test_CommitAgeWindow` / `test_CommitmentSpentOnRegister` | 커밋먼트 일치(이름·소유자·릴레이어), 60초/24시간 경계, 리빌 후 소진 |
| `test_A5_3_RecommitBeforeRevealMustNotResetWindow` | 감사의 정확한 공격 시퀀스: 공격자 재커밋 revert + 피해자 숙성 리빌 통과 + 소각 = 요금 |
| `test_A5_3_OnlyCommitterOrRelayerMayReveal` / `test_A5_3_CommitFrozenEvenForItsOwnCommitter` / `test_A5_3_RecommitAfterExpiryRebindsCommitter` | 타인 리빌 `NotCommitter`, 커미터 본인도 살아있는 슬롯 동결, 만료 후 재커밋은 새 커미터 것 |
| `test_RegisterViaDesignatedRelayer` | 해시 속 릴레이어만 대리 리빌(전액 요금, 소유자는 해시대로), 타인 `NotCommitter` |
| `test_FrontRunCannotStealPendingRegistration` | 리빌 복사·즉시 커밋-리빌 모두 실패 |
| `test_RegisterHappyPath` / `test_RegisterForAnotherAddress` / `test_RegisterTakenName` | 등록 이벤트·만료, 대납(지불자≠소유자, 커미터가 리빌), 중복 `NameTaken` |
| `test_ExpiryBoundaries` / `test_ReleasedNameRegistersFreshAndKeepsNoStaleData` | 만료 1초 전/직후/유예 마지막 초/해방 직후, 재등록 시 낡은 데이터 전멸 |
| `test_RenewalByAnyoneExtendsFromExpiry` / `test_RenewUnknownName` | 제3자 갱신·만료일 기준 연장, 미등록 `Unregistered` |
| `test_TransferTwoStep` / `test_TransferGuards` | 제안→승인, 오너 아닌 제안·승인, 취소, 덮어쓰기 |
| `test_SetAddrOwnerOnly` | 주소 기록 오너 전용 |
| `test_TextRecordBounds` / `test_TextRecordValidation` | 4키 상한·슬롯 회수·no-op 삭제, 키/값 크기·문자집합 |
| `test_ReverseRequiresPointback` / `test_ReverseCannotClaimSomeoneElsAddress` | 포인트백 없으면 거부, 남의 주소에 클레임 불가 |
| `test_ReverseSurvivesTransferUntilForwardMoves` / `test_ReverseOfUnclaimedAddress` | 이전 후에도 정방향이 유지되는 동안 클레임 유효, 미클레임 주소 |
| `test_ReentrantRefundCannotDoubleRegister` / `test_ReentrantRefundLegitimateSecondRegister` | 환급 중 재진입: 이중 등록 실패 / 별도 커밋의 정상 등록은 소각 총액 정확 |
| `test_MaliciousOwnerContractIsInert` | 컨트랙트 오너 전 사이클 — 오너 콜백 0회 |
| `testFuzz_StateInvariants` | 무작위 연산 흐름 뒤: 소각 총액 = 수수료 + 미공제 본드 = 소각 주소 잔액(컨트랙트 보관 0), 이름마다 오너 정확히 하나(모델 일치), 모든 유효 역방향은 정방향과 정합 (5000 런) |

테스트는 forge-std 없이 자체 `Vm` 인터페이스로 돕습니다 (저장소의 forge 0.2.0). 이 forge의 `vm.prank`/`expectRevert`/`expectEmit`은 **다음 외부 호출 하나**에만 적용되므로, 컨트랙트 뷰 읽기(예: `names.COMMIT_BOND()`)를 prank 이후 값 표현식에 두면 prank가 소모됩니다 — 테스트는 상수를 `setUp`에서 `bond`로 캐싱해 회피합니다. `FOUNDRY_FUZZ_RUNS=5000`으로 두 퍼즈 테스트가 각 5000 런을 돕니다.

## 의도적으로 만들지 않은 것

- **경매·가격 탐색** — 요금은 길이로 고정. 입찰·시세·종가 없음.
- **예약·프리미엄·허가 목록** — 특정 이름을 특별 취급하지 않음.
- **마켓플레이스·임대·담보** — 이름 매매는 소유자 간 2단계 이전으로 직접.
- **서브도메인** — `a.b` 형태 확장 없음. 레코드는 최상위 이름 하나에만.
- **수수료 수신자·수익 배분** — 받는 사람 자체가 없음. 전액 소각.
- **관리자 구제** — 실수로 잃은 이름의 복구 경로 없음. 유예 30일이 마지막 기회.

## 리네임과 CREATE2 주소 (감사 5차 A5-8)

이 컨트랙트는 CREATE2를 쓰지 않지만, 같은 시기의 Aether→EastSea 리네임이 형제 컨트랙트의 예측 주소를 바꿨으므로 여기에도 기록합니다(상세는 16-vault.md). 금고 팩토리와 Merkle distributor 팩토리는 컴파일러 메타데이터까지 포함된 자식 생성 코드의 해시로 CREATE2 주소를 계산하는데, 리네임이 그 메타데이터를 바꿔 예측 주소가 달라졌습니다. 클라이언트는 리네임 이전에 계산한 반사실적 금고 주소에 절대 선입금하지 말 것 — 옛 주소에 넣은 자금은 새 배포 경로로 도달할 수 없고, 새 메인넷에서 공표된 주소는 배포 후 수정이 불가능합니다.

## 남은 것 (나중)

- 지갑 앱 이름 화면(검색·커밋-리빌 등록·만료 알림·상호 확인 표시) — 이 문서의 이벤트/뷰 규격을 따름.
- 배포는 하지 않았다 (메인넷 전 승인 후).
