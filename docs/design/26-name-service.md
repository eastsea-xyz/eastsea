# 26. 이름 서비스 (.sea)

주소 대신 이름. 12-launch-plan.md E18 그대로: **고정 소각 요금, 경매 없음.** 오너·관리자·업그레이드·일시정지가 없는 불변 컨트랙트이고, 요금은 전액 소각됩니다 — 누구도 수수료를 받지 않습니다 (원칙: 비수탁, 수수료 0, 추천·순위 없음). 소유자는 주소이고 Aether 계정은 스마트 계정이므로 컨트랙트가 오너일 수 있습니다. 이 컨트랙트는 오너를 호출하지 않습니다: 소유권은 어떤 콜백 표면도 주지 않습니다.

소스: `contracts/src/EastSeaNames.sol` (`EastSeaNames`), 테스트: `contracts/test/EastSeaNames.t.sol`. toolbox 벤더 구현과 `IEastSeaNames`도 같은 API를 제공합니다.

2026-10-08 결정(0.7.4): 이름은 도메인 형태의 **`harbor.sea`**, 짧은 주소는 **`sea://harbor.sea/`**입니다. `sea://harbor`는 `harbor.sea`의 축약이고 `eastsea://`도 같은 의미입니다. 새 제네시스에는 `.sea`만 등록합니다. 옛 체인 **7780에서만** 클라이언트가 `.aeth`를 읽기 별칭으로 받을 수 있습니다. `.aeth`는 이 새 컨트랙트의 등록·노드 API에 입력하지 않고, 레거시 클라이언트가 기존 bare-label로 변환합니다.

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
| 호스트 문법 | RFC 1035/1123의 LDH, 레이블당 1–63바이트, 전체 호스트 ≤253바이트, 마지막 레이블은 `sea` | DNS 형태를 따르되 외부 DNS 조회를 하지 않음. 빈 레이블·마지막 점·대문자는 거부 |
| 등록 가능한 2단계 이름 | `.sea` 앞 레이블 3–32바이트 | 기존 등록·길이 요금 유지. `isValidHostname`의 문법 범위(1–63)와 `isValidName`의 등록 범위(3–32)를 구분 |
| 붙임표 | 각 레이블의 처음·끝 불가. 중간의 연속 붙임표는 허용 | `ab--c`도 LDH 문법에 맞음. `xn--` 문자열을 Unicode로 해석하는 IDN 기능은 아직 없음 |
| 이름 API | `harbor` 또는 `harbor.sea` | 기존 bare-label 호출과 해시 유지. 서브도메인은 `a.harbor.sea` 전체 이름만 받음 |
| 동작 호스트 예약 | `pay`, `call`, `connect`, `tx`, `app`, `follow`, `name`, `wallet`, `settings`, `send`, `receive`, `sign`, `deploy`, `open` | URI의 동작과 이름 충돌 방지. 추가 5개는 송수신·서명·배포·열기의 짧은 동작 별칭용. 해당 2단계 이름은 등록 불가; `pay.harbor.sea` 같은 자식 레이블은 가능 |
| 서브도메인 | `x.sea` 오너가 `any.x.sea`를 생성·삭제, 등록료·소각 0 | 각 부모 레코드가 있어야 하는 계층 구조. 오너는 주소 기록의 수신자와 구분; 자식에게 독립 소유권 이전을 하지 않음 |
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

짧은 이름은 희소하므로 비쌉니다 — 가격은 경매가 아니라 **2단계 레이블 길이**로 정해집니다(`abc.sea`는 3자 요금). URI 동작 호스트 외에 프리미엄·허가 목록 없음: 커밋-리빌 순서를 지킨 첫 사람이 이깁니다. 서브도메인 생성·삭제·기록 수정에는 이름 등록료를 받거나 소각하지 않습니다. 일반 트랜잭션 가스·상태 수수료는 체인의 공통 규칙대로 적용됩니다.

### 외부 TLD를 받지 않는 이유

창업자 2026-10-08: “.com이 필요없나? 실제 도메인이랑 헷갈릴까?” 출시에서는 **`.sea`만** 사용합니다. `.com`, `.xyz`, `.net` 등은 동해 이름으로 등록·해석하지 않습니다.

- **혼동·피싱:** 실제 웹사이트의 주소와 별개의 온체인 소유자를 같은 이름으로 표시하면 사용자가 기존 사이트의 신뢰를 잘못 가져옵니다.
- **통제권:** ICANN·등록기관의 갱신, 이전, 정지에 동해 이름 소유권을 종속시키지 않습니다.
- **DNSSEC 복잡도:** 증명 검증, 키 교체, 유효 기간, DNS와 온체인 권한의 수명 차이를 출시 경로에 더하지 않습니다.
- **출시 수요:** 외부 DNS를 가져와야 할 확인된 출시 요구가 없습니다.

지갑은 외부 TLD 입력에 **“웹 주소(.com 등)는 동해 이름이 아니에요. https://로 여세요.”**를 5개 지원 언어로 표시하고, 사용자가 `https://` 웹 주소로 열도록 제안합니다. DNS import는 가능한 미래 확장일 뿐 **계획하지 않습니다**. 별도의 DNSSEC 설계 문서는 만들지 않습니다.

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

### 서브도메인 수명과 계층 해석

`createSubdomain("a.harbor.sea", 주소)`·`deleteSubdomain("a.harbor.sea")`는 `harbor.sea` 오너만 호출합니다. `b.a.harbor.sea`를 만들려면 먼저 `a.harbor.sea`가 있어야 합니다. 주소·텍스트·역방향 API는 같은 전체 이름을 받으며, 모든 자식은 뿌리 이름의 오너가 관리합니다. 자식의 주소 수신자가 자식 관리 권한을 얻지는 않습니다.

- **유예는 뿌리의 갱신·소유권에만 적용합니다.** 부모 등록의 실제 만료 시각 `e`부터 모든 자식의 `ownerOf`·`addrOf`·`textOf`·`pendingOwnerOf`·`reverseOf`는 0/빈값을 돌려줍니다. 자식에 30일 유예를 더하지 않습니다.
- 각 자식은 부모 node와 생성 당시 부모의 **세대 번호**를 보관합니다. 해석할 때 뿌리까지 모든 부모의 존재·세대·만료를 확인합니다. 부모 삭제·재생성, 뿌리 재등록·이전, 만료 뒤의 유예 갱신은 세대를 바꿔 옛 자식을 영구 무효화합니다. 이전 뒤엔 새 오너가 자식을 다시 만들어야 합니다.
- 만료 전에 갱신하면 수명이 끊기지 않았으므로 기존 자식도 연장됩니다. 자식 `expiresOf`는 뿌리의 현재 만료일을 반환합니다.
- 조상만 다시 만들어도 낡은 손자는 살아나지 않습니다. 자식을 명시적으로 다시 만들 때 해당 노드의 옛 주소·텍스트·역방향 클레임을 초기화합니다. 하위 노드 전체를 순회하는 삭제·재등록이 없어 계층 무효화의 비용이 자식 수에 따라 늘지 않습니다.

### node와 커밋먼트 호환성

뿌리 node는 기존 해시를 유지합니다: **`nodeFor("harbor") == nodeFor("harbor.sea") == keccak256("harbor")`**. 서브도메인 node는 **`keccak256("a.harbor.sea")`**처럼 전체 소문자 호스트의 UTF-8 바이트를 해시합니다. ENS namehash나 재귀 해시로 바꾸지 않습니다. 부모 관계는 레코드에 명시해 계층을 확인합니다. `.aeth`·외부 TLD·잘못된 레이블의 `nodeFor`는 `InvalidName`입니다.

커밋먼트는 기존처럼 **호출에 사용한 이름 그대로** `keccak256(name ‖ owner ‖ salt ‖ relayer)`입니다. `harbor`로 커밋했으면 `harbor`로 리빌하고, `harbor.sea`로 커밋했으면 `harbor.sea`로 리빌해야 합니다. 두 표현은 같은 등록 node를 가리키므로 한쪽이 등록되면 다른 쪽도 `NameTaken`입니다. 커미터 결속(A5-3), 동결된 60초–24시간 창, 소각 본드(A5-1), 수수료·환급의 체크-이펙트-인터랙션은 유지합니다.

## 지갑이 이름을 읽는 법

| 방향 | 호출 | 뜻 |
|---|---|---|
| 정방향 (이름 → 주소) | `nodeFor(이름)` → `addrOf(node)` | 송금 주소. 해방된 이름은 0 |
| 역방향 (주소 → 이름) | `reverseOf(주소)` | 주소의 대표 이름 |
| 상호 확인 | `addrOf` 결과의 `reverseOf`가 같은 이름을 돌려주는지 | 이름이 그 주소의 것이 맞는지 확인 — UI는 통과할 때만 이름을 강조 |

이 단계는 이름의 `textOf(node, "app")` 레코드에서 appId를 읽고 [앱 레지스트리](31-app-registry.md)의 활성 릴리스 기록까지 조회합니다. `sea://`와 `eastsea://`는 같은 호스트를 해석합니다. 주소창 표시형은 `sea://<전체이름.sea>/…`입니다. 지갑은 해석된 앱 기록과 “content delivery comes next”를 보여줍니다. 콘텐츠 전달과 manifest의 양방향 `name_binding` 검증은 `ContentSource`를 구현하는 다음 lane의 작업이며, 검증 전에는 콘텐츠를 실행하거나 게시자 인증으로 표시하지 않습니다. 이름 해석만으로 결제·서명·컨트랙트 호출을 승인하지 않습니다.

역방향은 컨트랙트가 읽을 때마다 재검증합니다(살아있는 기록 + 정방향이 되돌아보기). 만료·주소 변경 뒤의 낡은 클레임과 뿌리 이전으로 무효화된 자식 클레임은 조용해집니다. 뿌리 자체의 기존 클레임은 이전 후에도 정방향 주소가 그대로인 동안 유지되며, `setAddr`은 옛 주소의 클레임을 즉시 회수합니다. 앱은 역방향 이름만으로 새 오너나 콘텐츠 게시자를 인증하지 않습니다.

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
| `SubdomainCreated(이름, node, parent, 오너)` | 무료 자식 생성; 주소 변경은 `AddrSet`도 발생 |
| `SubdomainDeleted(node, parent)` | 무료 자식 삭제; 후손은 세대 검증으로 무효 |

## 테스트 (`cd contracts && forge test`)

URL·주소창·기존 액션 호환성은 `tests/fixtures/sea-urls.json` 하나를 공유합니다. Swift는 `scripts/test-swift-pure.sh`, JavaScript는 탐색기·확장 `npm test`, Rust는 `cargo test -p aether-sea-url`로 검사합니다. Rust 파서는 클라이언트 전용 `crates/sea-url`에 둡니다. 이름 URL 해석 때문에 증명 게스트가 컴파일하는 `crates/types` 등이나 proving program id를 바꾸지 않습니다.

| 테스트 | 확인 |
|---|---|
| `test_NameGrammarRules` / `test_DNSHostnameGrammarRules` | 등록 길이·LDH 붙임표·대문자·예약어·외부 TLD, 레이블 63/64자·전체 253/254자 경계와 독립 문법 참조 |
| `test_CanonicalSeaNamesKeepBareLabelNodesAndFees` / `test_ReservedActionHostsCannotBeRegistered` | bare-label/`.sea` 동일 node·길이 요금·중복 등록, 모든 URI 예약어 등록 거부 |
| `test_SubdomainCreateResolveDeleteIsFreeAndHierarchical` / `test_SubdomainOnlyRootOwnerAndExistingParent` | 생성·다단계 해석·무료 소각·오너만 관리·부모 존재·삭제 뒤 조상 재생성에도 낡은 손자 미복원 |
| `test_SubdomainExpiryAtParentExpiryAndGraceRenewalCannotResurrect` / `test_SubdomainEarlyRenewalExtendsChildrenAndReregistrationSweepsThem` / `test_SubdomainTransferInvalidatesOldRecordsAndAuthority` | 실제 만료 경계·유예 갱신 뒤 미복원·만료 전 갱신·재등록·오너 이전과 낡은 역방향·하위 기록 차단 |
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
- **프리미엄·허가 목록** — URI 동작과 충돌하는 이름 외에는 특정 이름을 특별 취급하지 않음.
- **마켓플레이스·임대·담보** — 이름 매매는 소유자 간 2단계 이전으로 직접.
- **독립 서브도메인 등록·위임** — 뿌리 이름 오너가 무료 자식 기록을 관리. 자식을 따로 판매·등록·갱신하지 않음.
- **IDN** — Unicode 입력·정규화·퓨니코드 해석 없음. 후속 작업은 **UTS-46 → punycode 변환 + confusable 검사**를 함께 설계할 것. LDH인 `xn--…`는 지금은 의미를 해석하지 않는 ASCII 레이블일 뿐.
- **수수료 수신자·수익 배분** — 받는 사람 자체가 없음. 전액 소각.
- **관리자 구제** — 실수로 잃은 이름의 복구 경로 없음. 유예 30일이 마지막 기회.

## 리네임과 CREATE2 주소 (감사 5차 A5-8)

이 컨트랙트는 CREATE2를 쓰지 않지만, 같은 시기의 Aether→EastSea 리네임이 형제 컨트랙트의 예측 주소를 바꿨으므로 여기에도 기록합니다(상세는 16-vault.md). 금고 팩토리와 Merkle distributor 팩토리는 컴파일러 메타데이터까지 포함된 자식 생성 코드의 해시로 CREATE2 주소를 계산하는데, 리네임이 그 메타데이터를 바꿔 예측 주소가 달라졌습니다. 클라이언트는 리네임 이전에 계산한 반사실적 금고 주소에 절대 선입금하지 말 것 — 옛 주소에 넣은 자금은 새 배포 경로로 도달할 수 없고, 새 메인넷에서 공표된 주소는 배포 후 수정이 불가능합니다.

**0.7.4 주소 확인:** 이 문서/A5-8은 `EastSeaNames`의 고정 배포 주소를 지정하지 않았고, `crates/execution/src/predeploys.rs`의 제네시스 predeploy 목록도 CREATE2 deployer·Multicall3만 포함합니다. 이름 레지스트리가 이미 특정 주소에 predeploy된다는 근거는 없습니다. 이 lane은 그 배포 경로·레지스트리 설정이나 7780의 코드를 변경·배포하지 않습니다. 새 제네시스에서 이름 서비스를 설치할 때 이 `.sea` 구현을 기존 배포/레지스트리 선정 경로에 연결해야 합니다. **이미 배포된 불변 이름 컨트랙트에 이 기능을 적용하려면 재배포와 클라이언트 레지스트리 주소 갱신이 필요합니다**; 제자리 업그레이드는 없습니다. 실제 배포 주소가 정해지면 공표하고 검증할 것.

## 남은 것 (나중)

- 지갑 앱 이름 화면(검색·커밋-리빌 등록·만료 알림·상호 확인 표시) — 이 문서의 이벤트/뷰 규격을 따름.
- 배포는 하지 않았다 (메인넷 전 승인 후).
