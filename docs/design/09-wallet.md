# 09. 지갑 앱

## 구조

```
Aether.app (SwiftUI)
  ├─ Keys: CryptoKit SecureEnclave.P256.Signing (생체 정책), Keychain 메타
  ├─ UI: 잔액·송금·계약·상태 배지 / 설정(참여·증명 토글) / 엔지니어 모드(웹뷰 dashboard)
  ├─ Power: 전원·열·유휴 감시 → 역할 스위치
  └─ AetherCore.xcframework (UniFFI)
        └─ crates/ffi → aether-light(인증서·상태 증명 검증) + rpc client   (설계에 있던 proving::verifier, history downloader는 없음)
  Helpers: aether(노드, 별도 프로세스), aether-agent
```

## UniFFI 인터페이스 (실제: `crates/ffi`, proc-macro)

처음 설계한 `.udl` 파일과 `Node` 객체(`start`, `set_role`, `events`, `proof_state` 세 단계)는 만들지 않았다. 지금 FFI는 `uniffi::setup_scaffolding!()`과 `#[uniffi::export]` 함수, `#[derive(uniffi::Record)]` 구조체로 된 상태 없는 함수 모음이다. 노드는 FFI 안에서 돌지 않고, 앱이 번들한 `aether`를 별도 프로세스로 띄운다.

| 묶음 | 함수 (요지) |
|---|---|
| 연결·설정 | `configure_network`, `set_committee_identity`, `use_devnet_keys`, `use_local_node`, `local_node_height`, `connection`, `chain_status`, `verified_height` |
| 계정·잔액 | `account_address`, `verified_account`(확정 인증서 + EIP-7864 증명으로 검증, `aether-light`) |
| 송금·활동 | `prepare_transfer`, `prepare_batch`, `submit_signed`, `receipt`, `recent_blocks`, `account_history`(노드의 주소별 확정 내역, 커서 200건 이하), `devnet_faucet`(개발자 모드 전용) |
| 복구 | `recovery_key_code`, `prepare_set_recovery_key`, `prepare_add_recovery_key`, `recovery_status`, `prepare_recovery(_to)`, `prepare_recovery_submit`, `prepare_finish_recovery`, `prepare_cancel_recovery`, `prepare_remove_recovery_keys`, 복구 단어 `paper_key_new`·`paper_key_public`·`paper_key_sign` |
| 에이전트 세션 키 | `session_status`, `prepare_set_session`, `prepare_session_payment`, `prepare_session_submit` |
| 투표 노드 | `voting_node_status`, `prepare_register_node` |

- 서명은 Swift 쪽에서 한다(SE 키는 Rust로 나오지 않음). Rust는 서명할 바이트(`PreparedTx.signing_message`)를 만들고, Swift가 서명하면 low-s로 정규화해 제출한다.
- 검증 실패 닫힘(fail closed): 위원회 identity가 고정되지 않으면 모든 검증 API가 실패한다(`configure_network`도 오류). 공개 devnet 키는 명시적 개발 모드(`use_devnet_keys` 호출 또는 network.json의 `"devnet": true`)에서만 대신 쓸 수 있다. 검증된 앵커는 노드가 보고한 chain id와 블록이 담은 트랜잭션의 chain id 모두 지갑 설정값과 대조되고, 프로세스마다 "가장 높게 검증된 높이"(`verified_height`)를 넘지 못한다(되돌아가는 앵커 = 오래 된 상태의 재생). 10분 최신성 검사는 그 위에 추가로 유지된다.
- 없는 것: ZK 증명 검증(`verify_block`)과 "증명됨" 상태. 잔액 표시는 확정 인증서 검증 하나뿐이다(06-proving.md). 여러 가디언(k-of-n) 서명 수집과 `addOwner`도 FFI에 없다(12-launch-plan.md 4단계).

## 계정 모델

- 첫 실행: SE P-256 키 생성 → 주소 = 7702 위임 EOA(EOA 키는 SE 키에서 파생된 secp256k1 임시 키로 7702 서명 1회, 이후 P-256 소유자만 사용). 대안: 체인 자체 규칙으로 P-256 EOA 허용(EIP-8030 방식). **1단계에서 둘 중 택일**(EVM 도구 호환은 7702 방식이 유리).
- 위임 계약: P-256 소유자 검증(P256VERIFY), 세션 키(ERC-7715), 복구 서명자(선택), scheme 확장 슬롯.
- paymaster: 테스트넷 온보딩 가스 후원.

## 역할 정책

| 역할 | 기본 | 조건 | 자원 목표 |
|---|---|---|---|
| Verify | 켜짐 | 항상 | CPU ~0, RSS ≤200MB |
| Validate | 꺼짐 | 전원 연결 권장 | 블록당 순간 1~2코어 |
| Prove | 꺼짐 | 전원 연결 + 유휴 + 온도·배터리 임계 | GPU 100% 허용, 사용자 설정 상한 |

- Power 모듈이 macOS `IOPMCopyPowerSourcesInfo`, thermal state, idle time을 읽어 역할을 낮춘다. Advisor는 여기에 "언제 올릴지" 힌트만 준다.

## 온보딩 흐름

1. 설치(DMG) → 열기 → 생체 등록 → 키 생성 (설정 화면 없음)
2. 노드 id로 pkarr(DHT) 레코드를 찾아 iroh 연결 → 최신 인증서·증명 수신 → 내 계정 multiproof → "✓ 직접 검증됨"
3. 첫 화면: 잔액, 상태 배지, "네트워크에 기여하기" 카드(꺼짐)

## 서명 전 시뮬레이션

- 로컬 revm으로 tx 시뮬레이션 → 잔액 변화·로그·revert 표시 (Rabby 방식). 실패 예상 시 경고.

## 배포

- Sparkle(EdDSA 서명) + 노터라이즈 DMG. iOS는 4단계, Organization 계정 필요.
- 기존 개인 Team ID로 macOS 노터라이즈는 가능.

## 접근성·현지화

- 한국어·영어. 시스템 다크 모드. VoiceOver 라벨.


## 위임 계정(EIP-7702) — 구현됨 (2026-09-26)

- P-256(Secure Enclave) 계정에는 secp256k1 키가 없어 표준 7702 권한 튜플을 만들 수 없다. 대신 **자기 서명 tx 자체가 권한**이다: `EvmCall.delegate = Some(addr)`면 실행 전에 발신자 코드가 7702 지정자 `0xef0100‖addr`가 된다(`Address::ZERO`면 해제). 구현은 revm의 7702 처리에 발신자를 authority로 넣은 `RecoveredAuthorization`(nonce = tx nonce + 1, 표준 자기 후원 규칙)을 주는 방식이라 가스·nonce·EIP-3607 예외가 표준과 같다. 페이로드에는 선택적 꼬리(태그 0xd7 + 20B)로 붙어 기존 인코딩은 그대로 유효.
- 위임 대상 `AetherAccount`(`contracts/src/AetherAccount.sol`, 0x…7702에 제네시스 선배포): `execute((address,uint256,bytes)[])` — 여러 호출을 **서명 한 번(Touch ID 한 번)**에 원자적으로. 계정 자신만 호출 가능(`msg.sender == address(this)`).
- 지갑: 받는 사람을 쉼표로 여러 개 적으면 `prepare_batch`로 한 tx. 첫 배치에서 위임을 같이 설정하고 이후엔 생략.
- 상태: 코드 변경(위임 설정·교체·해제)을 트리에 기록(이전 코드 청크 삭제), BAL `code_touched`. 병렬 실행 차등 테스트에 위임·배치 연산 포함.
- 검증: 위임+배치 한 tx, 이후 배치, 해제, 타인 호출 거부(OnlySelf), 코덱 하위호환, 4검증자 devnet에서 서명 한 번으로 3곳 지급 후 경량 검증.

### 복구 키(가디언) — 구현됨

- `AetherAccount.setGuardian(x, y)`: 계정이 두 번째 기기(다른 맥·아이폰) Secure Enclave의 P-256 공개키를 복구 키로 등록(자기 호출만). 저장은 ERC-7201 네임스페이스 슬롯(`aether.account.guardian`) — 7702에서 저장소는 계정 자신의 것이라 충돌 방지.
- `guardianExecute(calls, r, s)`: 가디언 서명을 **P256VERIFY(0x100)**로 검증해 호출 실행. 서명 대상은 `sha256(abi.encode(chainid, account, nonce, calls))` — Secure Enclave가 SHA-256으로 서명하는 방식과 같다. nonce로 재생 방지, 누구나 중계 가능.
- 지갑(대칭 설계): "이 맥의 복구 키 코드"(x‖y) 복사 → 상대가 "내 복구 키로 지정". 키를 잃으면 가디언 맥에서 "분실 계정 복구": 잔액과 가디언 nonce를 **확정 인증서 + 저장소 증명으로 검증**한 뒤 가디언으로 서명, 자기 계정에서 중계(Touch ID 두 번).
- CLI `set-guardian`, `recover`. 검증: 복구 성공, 재생·다른 키·조작된 호출 거부, 가디언 없으면 불가, 4검증자 devnet에서 등록→복구→분실 계정 잔액 0 경량 검증.
- 다음: 세션 키(한도·기한 있는 위임 서명), 가디언 복수·지연(시간 잠금) 복구.


## Mac 앱 동작 — 구현됨 (2026-09-27)

- 첫 실행 때 버튼 없이 Secure Enclave 키를 만들고 바로 대시보드를 연다.
- 메뉴 막대에 산다. 창을 닫아도 앱과 노드는 계속 돈다. 메뉴 막대에 증명기 상태(마지막 증명, 밀린 블록 수 `lag`, 마지막 보상)가 보인다.
- 로그인 시 열기가 기본으로 켜져 있다(`SMAppService`). 설정에서 끌 수 있다.
- 업데이트(Sparkle)는 백그라운드에서 매시간 확인한다. 체인이 더 새 프로토콜을 예약하면 바로 확인한다.
- 결제 링크 `aether://pay?to=0x…&amount=1.5&memo=…&callback=https://…`: 웹 페이지가 확장 없이 결제를 요청한다. 앱은 송금 화면을 채워 보여 주고, 사람이 Touch ID로 승인해야 보낸다. 저절로 보내지 않는다. 결과(`tx`, `status`)는 콜백이 https일 때만 그 주소로 돌려준다. 빠진 값이 있는 링크는 무시한다.

## iOS 지갑 — 구현됨 (2026-09-26)

- 같은 Rust 코어(UniFFI)를 `aarch64-apple-ios`·`aarch64-apple-ios-sim`으로 빌드, 같은 SwiftUI 소스를 macOS·iOS 공용으로(`#if os(...)`로 배치·클립보드만 분기). iOS 타깃 `AetherWalletIOS`(iOS 17+, 아이폰). iroh의 iOS 경로 모니터 때문에 `Network.framework` 링크 필요.
- 키: 실기기는 Secure Enclave(Face ID/Touch ID/암호). **시뮬레이터에는 Secure Enclave가 없어** 소프트웨어 P-256 키를 쓰고 화면에 그렇게 표시.
- 검증: 시뮬레이터(iPhone 16, iOS 26.5)에서 Mainline DHT로 노드를 찾아 확정 인증서(BLS 임계 서명 1개) + EIP-7864 증명으로 잔액을 기기 스스로 검증. 실기기용 빌드(서명 없음) 성공.
- 이로써 아이폰이 맥 계정의 복구 키(가디언) 기기가 될 수 있다(복구 코드 교환).
- 빌드: `scripts/build-wallet.sh [macos|ios-sim|ios]`.
- 남은 것: 실기기 서명·배포(TestFlight는 Apple 계정·기기 등록 필요), App Store/Sparkle 배포.

## 토큰 보내기와 보안 표시 정책 — 구현됨 (2026-09-29)

앱과 확장 모두에 ERC-20 보내기와 token-spam-2026.md §6의 권고(팀장 채택 2026-09-28)를 넣었다. 순수 로직은 앱 `TokenSend.swift`·확장 `src/lib/safety.js`+`tokens.js`에 두고 양쪽이 같은 동작을 테스트로 맞춘다(`apps/wallet/Tests/token-send`, `apps/extension/test/send.test.mjs`).

### 토큰 보내기

- 보내기 화면에 자산 고르기(AETH + 보유 토큰, 항상 "SYMBOL · 0x8a9B…F41c" 표기 — 심볼만 표시하지 않는다). 자산 시트에서 토큰을 누르면 받는 사람 없이 보내기가 열린다.
- 금액은 십진 문자열을 정확히 파싱해 기본 단위로 바꾼다(부동소수점 없음, 토큰 decimals 초과 거부). calldata는 `transfer(address,uint256)` 셀렉터 `0xa9059cbb` + 단어 2개. 최대 버튼은 전체 자릿수로 채운다.
- 전송은 `prepare_call`(가스 한도 100,000; FFI에 estimateGas가 없어 고정)로 서명 — AETH와 같은 Touch ID, 같은 활동 기록("Sent 5 NEB to 0x12…ab"), 완결 후 잔액 새로 읽기.
- 확장은 같은 calldata를 기존 wasm `prepareTx` 경로로 보낸다(노드에서 잔액을 한 번 더 확인한 뒤).

### 표시 정책(제로 트러스트 수신함)

- 메인 자산 목록: AETH + 이 지갑의 **서명된 트랜잭션이 건드린** 토큰(보내기·승인·호출) + 공식 목록(token-sources.json seed + waeth) + 사용자가 직접 올린 토큰.
- 남이 보내기만 한 토큰은 접힌 "미확인(Unverified)" 섹션으로, 어떤 합계에도 들어가지 않는다.
- 런치패드 출처 토큰은 "Launchpad · unverified" 배지(카탈로그가 출처를 기억, 런치패드 > dex > pool > seed 우선). 공식 토큰 심볼·이름과 같거나 비슷하면(접기·편집거리 1) 사칭 경고.
- 가격·수익률은 어디에도 없다.

### 보내기 흐름 보호

- (a) 주소 오염: 받는 사람이 과거 송금 주소와 **앞 4 + 뒤 4 글자만** 같고 다른 주소면 차단 경고 — 전체 주소를 비교했다는 확인 전까지 보내기 버튼이 잠긴다.
- (b) 처음 보내는 주소에는 가벼운 안내.
- (c) 드라이런: 서명 전에 같은 전송을 보내는 사람 주소로 `eth_call`(상태 변경 없음). revert면 이유(Error(string) 디코딩)를 보여 주고 보내지 않는다. 앱은 로컬 노드 루프백(127.0.0.1:18545)으로 호출(FFI `eth_call`은 from을 못 줘서 토큰 전송이 무조건 revert한다); 노드가 없으면 AETH는 FFI `eth_call`으로 수신 불가 컨트랙트만 검사하고, 토큰은 "확인 못 함"으로 솔직하게 통과. 확장은 전체 드라이런.

### 저장 규칙

파생 집합("내가 보낸 주소", "내 트랜잭션이 건드린 토큰")은 지갑이 이미 추적하던 활동/영수증에서 계산한다(체인에 아무것도 새로 쓰지 않는다). 기기에만 저장하는 것은 사용자 선택(숨김·표시, UserDefaults `tokenChoices.<chain>` / `chrome.storage.local`)이다. 드라이런 결과는 저장하지 않는다. 도우미 상단 주석과 앱 보안 화면에 이 문장을 뒀다: "이 검사는 이 기기의 공개 체인 데이터와 설정만 읽는다. 체인에는 아무것도 새로 쓰지 않는다."

### 주소별 확정 내역과 연결 지갑

- 노드는 확정 블록의 송신자·네이티브 AETH 수신자·ERC-20 `Transfer` 로그의 송수신자·노드/증명 보상 수령자를 주소별로 색인한다. 위임 계정의 `execute` 배치도 성공한 내부 송금의 모든 수신자를 색인하며, 같은 거래에서 같은 주소로 여러 번 보냈다면 금액을 합쳐 한 행으로 둔다. `aether_accountHistory [address, before_cursor?, limit]`는 최신순으로 돌려주며 `history_start`와 `indexed_height`를 함께 보낸다. 기존 저장소를 업그레이드한 노드는 색인을 시작한 높이부터 표시한다. 가지치기한 블록의 색인도 함께 지운다.
- 앱은 FFI의 읽기 분산 경로로 이 데이터를 읽고, 로컬 대기 거래와 해시·주소 쌍으로 합친다. 한 거래가 연결 지갑 두 주소를 건드리면 두 활동 항목을 모두 보여 준다. 상세 정보에는 **"From the node"**를 표시한다. 이 내역은 노드의 표시 데이터이며 AETH 잔액은 계속 확정 인증서와 상태 증명으로 검증한다.
- DEX Router·런치패드·TokenFactory는 배포 주소와 메서드 셀렉터가 함께 맞을 때 스왑·유동성·출시로 해석한다. 스왑은 Pair `Swap` 로그가 있어야 확정 스왑으로 표시하고, 수량은 트랜잭션 값과 `Transfer`/WAETH `Withdrawal` 로그에서 읽는다. ERC-20 토큰은 심볼만 쓰지 않고 주소를 곁들인다. 알 수 없는 호출도 메서드와 `Transfer` 로그의 토큰 이동을 보여 준다. 연결 지갑은 공개 주소만 보관하고, 서명 키나 권한은 합치지 않는다.
- 새 수신 거래 알림은 주소별 마지막 처리 높이와 해시로 한 번만 보낸다. 검증된 잔액이 인접한 블록에서 증가했는데 수신 내역이 없으면 그 높이에 잔액 증가 항목을 표시한다. 내부 AETH 이동은 별도 로그가 없으므로 이 대체 항목은 관측한 높이에 한정된다.
- 테스트넷 faucet은 앱 심플 모드와 확장 홈에 두지 않는다. 확장에서는 설정의 개발자 모드를 켠 뒤에만 보인다.

### 남은 것

- 앱의 토큰 드라이런은 로컬 노드가 있을 때만 완전하다(아이폰은 미확인 통과).

## 리소스 설정 — 구현됨 (2026-09-29)

설정 ▸ 리소스(맥 앱). 배경과 노드 쪽 동작은 [docs/ops/resource-limits.md](../ops/resource-limits.md) — 2026-09-29 사고(증명 사이드카 14 GB·스왑 95%)로 생겼다.

- **증명(Prover) 사용** 토글: 예전의 "Prove blocks with Metal on this Mac's GPU" 토글을 이 자리로 옮겼다. 켤 때 지갑 주소를 증명 보상 주소로 묶는 것도 그대로.
- **최대 메모리**: 자동 (RAM의 25%) / 4 GB / 8 GB / 16 GB / 끄기 → `--prover-max-memory=auto(생략)/4/8/16/0`. 끄기는 증명 자체를 끈다.
- **최대 CPU**: 절반(기본, 생략) / 전부 → `--prover-threads=<코어 수>`.
- **배터리에서 증명 허용**: 끔이 기본. 켜면 `--prover-on-battery`.
- 상태 줄: 증명 사이드카의 현재 메모리(`aether_proverStatus`의 `memory_bytes`/`memory_cap`), `paused == "memory"`면 "메모리 부족으로 일시 정지", `aether_status`의 `resources.disk_low`면 "디스크 공간 부족".

선택은 UserDefaults(`proverMemory`·`proverCores`·`proverOnBattery`)에 저장되고, 노드를 (재)시작할 때 `ProverFlags.build`가 플래그로 만들어 `aether run`에 붙인다. 기본값(자동·절반·배터리 거부)은 플래그를 하나도 안 붙인다 — 노드 스스로 안전한 기본값(RAM의 25%, 코어의 절반)을 고르고, 플래그를 모르는 옛 노드 번들이어도 시작에 실패하지 않는다. **심플 모드에는 예산 조절 UI가 없다**(개발자 모드에서만 보인다): 노드의 안전한 기본값이 그대로 적용되고, 증명 토글과 경고("메모리 부족으로 일시 정지"·"디스크 공간 부족")는 심플 모드에서도 보인다 — 전기·발열을 쓰는 스위치를 숨기거나 문제를 조용히 넘기지 않는다. 메뉴 막대의 증명 상태에는 일시 정지 이유가 영어로 한 줄 더 붙는다.
