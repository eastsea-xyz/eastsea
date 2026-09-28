# 16. 금고 (Vault)

멀티시그·가족 계정을 하나로 합친 공용 금고. 12-launch-plan.md 원칙 그대로: **비수탁, 수수료 0, 불변·무관리자 컨트랙트.** 오너는 사람의 주소가 아니라 P-256 공개 키(Mac·iPhone Secure Enclave, 가족·팀 멤버)이고, M-of-N 정족수가 자금을 지킨다. Squads/Safe 형태를 Aether답게: 출금은 24~48시간 대기 중 취소 가능, 소액은 하루 한도로 즉시(12-launch-plan.md "금고" 항목, docs/research/popular-utilities-2026.md 1)·2) 참고).

소스: `contracts/src/AetherVault.sol` (`AetherVault`, `AetherVaultFactory`), 테스트: `contracts/test/AetherVault.t.sol`. Rust 변경 없음.

## 모델

```
오너(=SE 키) M-of-N (1 ≤ M ≤ N ≤ 8; AetherAccount 키 한도와 동일)
  │
  ├─ spend:        오너 1명 서명 → 네이티브 AETH 즉시 송금 (하루 한도 안)
  │                 한도는 금고 전체 기준, 임의의 24시간 창에 적용
  └─ 대기열(제안):  그 외 전부(한도 초과액, ERC-20)
        제안(propose, 첫 승인 포함) → 승인(approve) M개 →
        정족수 도달 시각 + delay(기본 48h, 최소 24h 강제) 후 execute
        취소(cancel): 오너 아무나 1명 서명으로 정족수 전·대기 중 언제든
설정 변경(오너 교체·정족수·한도·지연)도 같은 대기열 경로를 통해서만
```

- 오너는 키이므로 트랜잭션을 직접 보낼 수 없다. 모든 행동은 **서명 + 릴레이**다: 오너가 다이제스트에 서명하면 누구나(보통 지갑 앱) 그것을 전달한다. `propose`·`approve`·`cancel`·`execute`·`spend` 모두 무차별 허용, 권한은 서명 검증에만 있다.
- 승인은 서명 다이제스트가 모두 같으므로 순서·시점 무관. 정족수에 도달한 뒤 추가 승인은 `readyAt`를 연장하지 않는다.
- 설정 제안은 **전체 설정을 통째로 교체**한다(앱은 체인상 현재 설정을 읽어 한 필드만 바꿔 제출). 실행 시 대기열 전체가 새 시대(queue era)로 무효화된다 — 옛 오너 세트에 대한 승인이 새 오너 세트 아래에서 살아남으면 안 되기 때문이다. 무효화된 제안의 스토리지는 남지만 `SettingsExecuted(id, era)` 이벤트로 앱이 목록을 정리한다.
- 즉시 지출(`spend`)은 네이티브 AETH 순수 송금만 허용한다(콜데이터 없음). ERC-20 출금은 언제나 대기열 경로 — 토큰 컨트랙트 코드가 단일 서명만으로 실행되는 일을 원천적으로 막는다. ERC-20 일일 한도는 v1에서 의도적으로 뺐다(토큰별 단위 문제, 단순성).

## 서명 (AetherAccount 방식 재사용)

P256VERIFY 프리컴파일(0x100)로 검증하는 SHA-256 다이제스트. EIP-712형 도메인: **체인 id + 금고 주소 + 태그 + nonce**가 모든 다이제스트에 들어가 금고 간·체인 간 재생을 차단한다.

| 행동 | 태그(keccak256) | nonce | 페이로드 |
|---|---|---|---|
| 즉시 지출 `spend` | `aether.vault.spend` | `spendNonce`(건마다 +1) | to, amount |
| 출금 제안·승인 | `aether.vault.withdraw` | 제안 id(1부터, 재사용 없음) | token(0=네이티브), to, amount |
| 설정 제안·승인 | `aether.vault.settings` | 제안 id | 오너 목록, 정족수, 한도, 지연 |
| 취소 `cancel` | `aether.vault.cancel` | `cancelNonce`(건마다 +1) | 제안 id |

승인(`approve`)은 제안 구조체에 저장된 필드로 다이제스트를 다시 계산해 검증하므로, 승인 시점에 원래 파라미터를 다시 전달할 필요가 없다(서명이 서로 다른 파라미터에 대한 것이면 `BadSignature`).

일일 한도 계산은 AetherAccount 세션 키와 같은 트릭: 임의의 24시간 창은 연속한 두 UTC 하루를 넘지 못하므로, "오늘 + 어제" 지출 합을 한도로 묶으면 모든 창이 묶인다. `dailyAvailable()` 뷰가 앱에 남은 한도를 준다.

## 보안 설계

- **재진입**: 모든 상태(논스·한도·제안 삭제·시대)를 외부 호출 전에 정리한다(체크-이펙트-인터랙션). ERC-20 `transfer` 안에서 같은 제안의 `execute`를 다시 부르면 이미 삭제돼 `UnknownProposal`로 실패한다 — 테스트로 확인.
- **재생**: 위 표의 논스 구조. 같은 서명의 재사용(`spend` 재생, 승인 이중 반영, 취소 서명 재사용, 다른 금고·다른 체인에서의 재생)은 모두 테스트로 확인.
- **설정 불변식**: 생성 시와 설정 제안 시 같은 검증(오너 1~8명, 키 0·중복 금지, 1 ≤ 정족수 ≤ 오너 수, 지연 ≥ 24h). 설정 제안은 검증된 값이 구조체에 그대로 저장되므로 실행 시 재검증이 필요 없다.
- 이체 실패 시 트랜잭션 전체가 되돌아가 제안이 그대로 남는다(재시도 가능).
- 관리자·업그레이드·수수료 경로는 아예 없다. `receive()`로 AETH를 보관하고, ERC-20은 그냥 금고 주소로 전송하면 된다.

## 이벤트 (앱이 대기 중 출금 표시·알림에 씀)

| 이벤트 | 뜻 |
|---|---|
| `WithdrawalProposed(id, token, to, amount)` | 출금 제안 등장 |
| `SettingsProposed(id, newOwners, …)` | 설정 변경 제안 등장 |
| `Approved(id, ownerIndex, approvals, readyAt)` | 승인 추가. `readyAt != 0`이면 이 승인으로 정족수 도달(실행 예정 시각 = 알림 시점) |
| `WithdrawalExecuted(id, token, to, amount)` | 출금 실행 |
| `SettingsExecuted(id, era)` | 설정 적용. 이전 시대 제안은 모두 무효 |
| `Canceled(id, ownerIndex)` | 제안 취소 |
| `Spent(nonce, to, amount, ownerIndex, spentToday)` | 한도 내 즉시 지출 |
| `VaultCreated(vault, salt, threshold, dailyLimit, delay)` | 팩토리 배포 |

## 팩토리 (CREATE2)

`AetherVaultFactory.create(keys, threshold, dailyLimit, delay, salt)`는 `new AetherVault{salt}`로 배포하고, `predict(...)`가 같은 인자에 대한 주소를 미리 계산한다. 주소는 오너·정족수·한도·지연(생성 코드에 인코딩) + salt + 팩토리 주소의 함수로 결정적 — 앱은 배포 전 주소를 보여 줄 수 있고 어떤 기기에서든 다시 유도할 수 있다. 같은 인자+salt의 재배포는 CREATE2 충돌로 되돌아간다. 프록시가 아니라 풀 계약을 그대로 배포한다(불변 원칙에 부합, Aether에서 가스는 싸다).

## 테스트 (`cd contracts && forge test`)

| 테스트 | 확인 |
|---|---|
| `test_SpendWithinDailyLimitByOneOwner` | 오너 1명 서명 즉시 지출, 한도는 금고 전체 공유, 초과 `OverDailyLimit`, `Spent` 이벤트 |
| `test_DailyLimitSlidingWindow` | 같은 UTC 하루, 다음 날(어제 지출 반영), 이틀 후 창 이동 |
| `test_WithdrawalQueueApprovalsDelayExecute` | 정족수 미달 `NotReady`, 정족수 즉시 `readyAt`, 추가 승인 연장 없음, 47h `NotYet`, 48h 실행·재실행 `UnknownProposal` |
| `test_ERC20WithdrawalThroughQueue` | ERC-20 출금 대기열 경로 |
| `test_CancelByAnyOwner` | 정족수 전 취소, 대기 중 취소(제안자가 아닌 오너), 설정 제안 취소 |
| `test_OwnerRotation` | 설정 제안 M-of-N+지연 후 교체, 옛 대기열 무효화, 제외된 오너 서명 거부, 새 오너 지출·제안 |
| `test_ReplaySpendNonce` / `test_ReplayApprovalTwice` / `test_ReplayCancelNonce` | 논스·승인 비트마스크 재생 차단 |
| `test_ReplayAcrossVaults` / `test_ReplayAcrossChains` | 금고 주소·체인 id 바인딩 |
| `test_ReentrancyOnERC20Transfer` | 악성 토큰의 `transfer` 중 재진입 실패, 자금은 정확히 한 번 이동 |
| `test_WrongSignerRejected` | 오너 아닌 키, 서명자·인덱스 불일치, 파라미터 변조, 범위 밖 인덱스 |
| `test_DelayMinimumEnforced` | 생성(직접·팩토리) 및 설정 제안에서 24h 미만 `BadConfig`, 24h 정확히는 허용 |
| `test_ConfigValidation` | 정족수 0/초과, 오너 없음, 중복 키, 0 키 |
| `test_FactoryDeterministicAddress` / `test_VaultCreatedEvent` | `predict` == 실제 주소, 충돌 revert, salt·파라미터별 주소 변화 |
| `test_P256KnownVector` | 테스트용 P-256 라이브러리를 파이썬으로 생성한 독립 벡터에 고정 |

테스트는 forge-std 없이 자체 `Vm` 인터페이스로 돈다. 이 저장소의 forge(0.2.0) EVM에는 RIP-7212 프리컴파일이 없어서 `setUp`에서 0x100에 동일 입출력(160바이트 입력 → 32바이트 1/0)의 솔리디티 검증기를 `vm.etch`한다. 실제 체인에서는 진짜 프리컴파일이 그 자리를 쓴다. 서명·검증 산술은 전부 테스트 안의 `LibP256`(야코비안 좌표, modexp 프리컴파일로 역원)이고, `test_P256KnownVector`가 이를 외부 벡터와 대조한다.

## 남은 것 (나중)

- 지갑 앱·확장 금고 화면(생성, 대기 중 제안 목록·알림, 승인·취소 서명 플로우) — 이 문서의 이벤트/다이제스트 규격을 따름.
- ERC-20 일일 한도(토큰별 한도 테이블), 가스 대낭(E5-lite)은 필요 시 별도 설계.
- 배포는 하지 않았다(메인넷 전 승인 후).
