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
| 연결·설정 | `configure_network`, `set_committee_identity`, `use_local_node`, `local_node_height`, `connection`, `chain_status` |
| 계정·잔액 | `account_address`, `verified_account`(확정 인증서 + EIP-7864 증명으로 검증, `aether-light`) |
| 송금 | `prepare_transfer`, `prepare_batch`, `submit_signed`, `receipt`, `recent_blocks`, `devnet_faucet` |
| 복구 | `recovery_key_code`, `prepare_set_recovery_key`, `prepare_add_recovery_key`, `recovery_status`, `prepare_recovery(_to)`, `prepare_recovery_submit`, `prepare_finish_recovery`, `prepare_cancel_recovery`, `prepare_remove_recovery_keys`, 복구 단어 `paper_key_new`·`paper_key_public`·`paper_key_sign` |
| 에이전트 세션 키 | `session_status`, `prepare_set_session`, `prepare_session_payment`, `prepare_session_submit` |
| 투표 노드 | `voting_node_status`, `prepare_register_node` |

- 서명은 Swift 쪽에서 한다(SE 키는 Rust로 나오지 않음). Rust는 서명할 바이트(`PreparedTx.signing_message`)를 만들고, Swift가 서명하면 low-s로 정규화해 제출한다.
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
2. Pkarr로 부트노드 → iroh 연결 → 최신 인증서·증명 수신 → 내 계정 multiproof → "✓ 직접 검증됨"
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
