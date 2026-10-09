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

## 영수증 인증

새 제네시스(`node_rewards || history_v2`)의 높이 1부터 블록은 현재 블록 실행 영수증의 `receipts_root`를 인증서와 함께 확정한다. 영수증 응답의 트랜잭션 인덱스·BLAKE3 머클 경로와 영수증 전체(성공 여부, 가스, 로그, 출력 등)를 정규 인코딩해 인증된 블록 루트에 대조하면 개별 실행 결과를 검증할 수 있다. leaf/node/root 해시는 도메인을 분리한다. 제안자는 제안 전에 실행하고 검증자는 재실행하므로 루트가 현재 블록 결과에 묶인다.

`aether_getReceiptProof(tx_hash)`는 영수증, 블록 내 인덱스, 경로, 같은 높이의 인증 블록을 함께 준다. 브라우저의 `verifyReceipt`는 인증서·체인·트랜잭션 해시·경로를 검사한다. 기존 `aether_getReceipt` 응답 형식은 유지한다.
프루닝으로 해당 영수증이나 인증서를 보관하지 않는 노드는 증명을 제공할 수 없으며, 이 경우 보관 노드에 질의한다.
과거 영수증의 포함 증명은 만료되지 않으므로, 잔액 읽기와 달리 블록 타임스탬프에 10분 최신성 제한을 적용하지 않는다.
과거 거래를 조회할 때 `verifyReceipt`의 `minimum_height`에는 해당 조회에 필요한 최소 높이(제약이 없으면 0)를 준다. 잔액 확인에 쓰는 최신 높이를 그대로 넘기면 이전 블록 영수증은 의도대로 거부된다.

개별 로그의 포함은 해당 영수증 증명으로 확인한다. `eth_getLogs`가 반환한 범위에 빠진 로그가 없다는 완전성은 별도 범위 증명이 필요하며 아직 제공하지 않는다.

테스트넷 7780에는 이 인증을 소급 적용하지 않는다. 기존 영수증·주소별 내역은 노드의 표시 데이터이며, 기존 블록과 커밋 바이트 및 proving guest program ID는 유지한다. 현재 영수증 루트는 합의 재실행과 인증서로 보호되며 BlockProof 명제에는 포함되지 않는다. 나중에 게스트에 추가하면 program ID가 바뀌므로 별도 업그레이드가 필요하다.

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
- 위임 대상 `EastSeaAccount`(`contracts/src/EastSeaAccount.sol`, 0x…7702에 제네시스 선배포): `execute((address,uint256,bytes)[])` — 여러 호출을 **서명 한 번(Touch ID 한 번)**에 원자적으로. 계정 자신만 호출 가능(`msg.sender == address(this)`).
- 지갑: 받는 사람을 쉼표로 여러 개 적으면 `prepare_batch`로 한 tx. 첫 배치에서 위임을 같이 설정하고 이후엔 생략.
- 상태: 코드 변경(위임 설정·교체·해제)을 트리에 기록(이전 코드 청크 삭제), BAL `code_touched`. 병렬 실행 차등 테스트에 위임·배치 연산 포함.
- 검증: 위임+배치 한 tx, 이후 배치, 해제, 타인 호출 거부(OnlySelf), 코덱 하위호환, 4검증자 devnet에서 서명 한 번으로 3곳 지급 후 경량 검증.

### 복구 키(가디언) — 구현됨

- `EastSeaAccount.setGuardian(x, y)`: 계정이 두 번째 기기(다른 맥·아이폰) Secure Enclave의 P-256 공개키를 복구 키로 등록(자기 호출만). 저장은 ERC-7201 네임스페이스 슬롯(`aether.account.guardian`) — 7702에서 저장소는 계정 자신의 것이라 충돌 방지.
- `guardianExecute(calls, r, s)`: 가디언 서명을 **P256VERIFY(0x100)**로 검증해 호출 실행. 서명 대상은 `sha256(abi.encode(chainid, account, nonce, calls))` — Secure Enclave가 SHA-256으로 서명하는 방식과 같다. nonce로 재생 방지, 누구나 중계 가능.
- 지갑(대칭 설계): "이 맥의 복구 키 코드"(x‖y) 복사 → 상대가 "내 복구 키로 지정". 키를 잃으면 가디언 맥에서 "분실 계정 복구": 잔액과 가디언 nonce를 **확정 인증서 + 저장소 증명으로 검증**한 뒤 가디언으로 서명, 자기 계정에서 중계(Touch ID 두 번).
- CLI `set-guardian`, `recover`. 검증: 복구 성공, 재생·다른 키·조작된 호출 거부, 가디언 없으면 불가, 4검증자 devnet에서 등록→복구→분실 계정 잔액 0 경량 검증.
- 다음: 세션 키(한도·기한 있는 위임 서명), 가디언 복수·지연(시간 잠금) 복구.

### 원래 키는 복구로 폐기되지 않는다(F-05, 감사 2026-10-05)

- 이 구조의 대가다. EIP-7702 위임은 계정을 다른 코드로 갈아끼우거나 해제할 권한을 **원래 EOA 서명자**(첫 실행 때 SE P-256 키에서 파생한 secp256k1 임시 키)에게 남긴다. 가디언 복구·소유자 키 추가는 잃어버린 P-256 소유자를 구하지만, 도난당한 원래 키의 이 권한을 거두지는 못한다. 원래 키는 첫 위임 서명 한 번 뒤 다시 쓰이지 않아 평소 공격 면이 작지만, 한 번 노출되면 그 계정의 권한은 사실상 두 쪽이다(`EastSeaAccount.sol`의 자기 호출 검증·ERC-7201 슬롯 분리는 제3자 초기화·충돌은 막아도 이 경로는 막지 못한다).
- 지갑이 할 일: (1) "분실 계정 복구" 화면과 안내 문서에 이 한계를 숨기지 않는다 — 가디언 복구는 잃은 키의 회복이지 도난한 키의 폐기가 아니다. (2) 원래 키가 노출됐을 수 있다는 판단이 서는 순간(파생 키가 살던 맥을 오래 열어 둔 경우, 백업 유출 등) 사용자에게 **잔액 전부를 새 주소로 옮기도록** 안내하고, 그 계정의 위임 변경·자금 이동을 감시한다. (3) 어떤 설명에서도 "가디언이 있으면 키 도난을 되돌린다"고 주장하지 않는다.
- 반영(2026-10-06): Mac 지갑은 복구 패널(간단 화면·개발자 화면)과 키 보관 카드("Protected by this device")에 "복구는 원래 키를 막지 못한다 — 원래 키가 노출됐을 수 있으면 잔액 전부를 새 계정으로 옮기라"는 한·영 안내를 띄운다(`KeyExposureNotice`, 순수 Swift 테스트 `key-exposure`). 키 내보내기는 Mac 지갑에 없다(SE 키는 밖으로 나가지 않는다).
- 완전한 폐기는 이 설계 밖이다. 계정 권한 구조 자체를 바꿔야 하므로 새 위임 대상 설계에서 다루고, 그렇게 해도 이미 위임된 기존 계정은 소급되지 않는다.


### ERC-1271 서명·토큰 수신 훅 — 새 제네시스 (2026-10-06, 클론 카탈로그 §0 B1·B2)

새 제네시스의 위임 대상 코드(`aether_account_v2.bin.hex`, 0x…7702)에만 들어간다. 7780의 계정 코드는 바이트 단위로 그대로다.

- **`isValidSignature(bytes32 hash, bytes sig)` → `0x1626ba7e`**: Permit2·Seaport·OpenZeppelin `SignatureChecker`처럼 계정의 "서명"을 묻는 컨트랙트에 답한다. `sig`는 `r ‖ s ‖ x ‖ y`(128바이트) P-256 서명이고 검증은 P256VERIFY(0x100). 형식이 틀리거나 무효면 되돌리지 않고 `0xffffffff`.
- **서명 대상(계정·체인 바인딩)**: 키는 `hash`를 그대로 서명하지 않는다. EIP-712 메시지 `"\x19\x01" ‖ domainSeparator ‖ keccak256(abi.encode(keccak256("Contents(bytes32 contents)"), hash))`를 서명하며, 도메인은 `{name "EastSeaAccount", version "2", chainId, verifyingContract = 계정 주소}`다. P256VERIFY에 넣는 다이제스트는 이 66바이트의 SHA-256 — 다른 계정 서명과 같이 Secure Enclave `signature(for: message)`가 그대로 만든다(`signatureMessage(hash)`·`signatureDigest(hash)` 뷰로 확인). 그래서 같은 키가 소유한 두 계정 사이, 두 체인 사이에서 서명을 재사용할 수 없다.
- **누가 서명할 수 있나(기본 거부)**: 소유자 키만. (1) 계정의 원래 SE 키 — 체인 주소 `keccak256(0x01 ‖ 압축 공개키)[12:]`(`aether_crypto::address_of`)가 이 계정인 키, 등록 불필요; (2) 복구가 추가한 소유자 키(`owners`) — 이미 `ownerExecute`로 무엇이든 할 수 있으므로 새 권한이 아니다. **세션 키(AI 비서 포함)와 가디언은 언제나 거부**한다: 세션 키의 권한은 한도 안의 지급뿐이고 어떤 세션 한도도 메시지 서명을 허락하지 않으며, 가디언은 지연 복구 제안만 한다. 1271 서명은 Permit2 승인처럼 한도 밖에서 토큰을 옮기게 할 수 있으므로 세션 키에 주면 한도가 무의미해진다. 세션에 이 권한을 주려면 별도 명시 한도를 설계해야 한다(현재 없음).
- **low-s만**: 체인의 다른 P-256 서명과 같이 `s ≤ n/2`만 받는다(같은 메시지에 서명이 하나뿐).
- **구형 셀렉터** `isValidSignature(bytes data, bytes sig)` → `0x20c13b0b`: `data`가 정확히 32바이트(해시)일 때만 표준형과 같은 검사, 그 밖의 길이는 무효 — 두 형식이 서명 대상을 다르게 해석할 수 없다.
- **토큰 수신 훅**: `onERC721Received`·`onERC1155Received`·`onERC1155BatchReceived`가 각자 셀렉터를 돌려준다 — 위임 계정도 `safeTransferFrom`·`_safeMint`·ERC-1155 전송을 받는다(위임 뒤엔 코드가 있어 훅이 없으면 거부됐다). 받기는 지출이 아니므로 상태 변경·권한 없음. ERC-165 `supportsInterface`: `0x01ffc9a7`(165), `0x1626ba7e`(1271), `0x150b7a02`(721 수신), `0x4e2312e0`(1155 수신).
- 검증: Foundry `EastSeaAccount1271.t.sol`(실제 P256VERIFY, 원래 키·소유자 키 유효, 다른 해시·잘린 서명·high-s·다른 계정·다른 체인·세션·가디언·구형 셀렉터), 툴박스 `AccountReceiver.t.sol`(OZ ERC721/1155 safe 전송, 고정된 v2 바이트 그대로), Rust 실행기 `contracts_onchain::account`(7702 위임 P-256 계정이 NFT를 `safeTransferFrom`으로 받고, OZ `SignatureChecker.isValidSignatureNow`가 그 계정 키의 1271 서명만 받는다 — 주소 파생을 `address_of`와 대조).

### dApp 서명 검토와 계정 이전 (E5b·E7b)

- 앱의 Explore 제공자와 브라우저 확장은 거래 승인 전에 실제 보내는 주소로 `eth_call`, `eth_estimateGas`, 읽기 전용 `aether_simulateTransaction`을 실행한다. 호출 대상, 수수료와 별개인 잔액 변화, 토큰별 이동, 지출 승인, 실행 실패 이유를 보여 준다. ERC-20 변화는 실행 전후 잔액을 우선하고, 읽지 못한 토큰·계약 이벤트·해석하지 못한 효과는 한계를 표시한다. 상태는 쓰지 않으며 미리보기는 포함 시점의 결과를 보장하지 않는다.
- 실행 실패가 예상되는 거래는 별도 확인을 요구한다. 시뮬레이션 자체를 읽을 수 없으면 서명하지 않는다. 서명 직전에 결과를 다시 비교하고, 결과·계정·체인·사이트 권한·승인 요청이 달라졌으면 다시 검토한다.
- `eth_signTypedData_v4`는 도메인·선언된 계약·체인·중첩 필드를 읽기 쉬운 형태로 보여 준다. 큰 정수와 주소는 그대로 표시하고 바이트는 길이와 펼쳐 보기로 보여 준다. 명시적인 도메인 `chainId`가 현재 체인과 다르면 거부한다. 실제 계정의 위임과 배포된 정본 v2 코드를 확인하고 위의 계정·체인 바인딩 ERC-1271 형식으로 답한다. 메시지 서명은 거래를 제출하지 않는다.
- 보안의 **계정 업그레이드와 이전**은 정본 계정 코드로 같은 주소를 재위임하거나, 새 주소로 자산별 일반 송금 검토를 연다. 7780의 구형 코드는 업그레이드 대상이 아니다. 토큰을 먼저 옮기고 남은 네이티브 잔액에서 수수료를 뺀 정확한 금액을 마지막으로 옮긴다. 각 검토는 보낸 계정·체인·승인 수명에 묶인다. 업그레이드·가디언 복구는 노출된 원래 키를 폐기하지 않으며, NFT·승인·주문·세션·저장된 지급 주소·앞으로 들어올 입금은 따로 확인해야 한다.

## Mac 앱 동작 — 구현됨 (2026-09-27)

- 첫 실행 때 버튼 없이 Secure Enclave 키를 만들고 바로 대시보드를 연다.
- 메뉴 막대에 산다. 창을 닫아도 앱과 노드는 계속 돈다. 메뉴 막대에 증명기 상태(마지막 증명, 밀린 블록 수 `lag`, 마지막 보상)가 보인다.
- 로그인 시 열기가 기본으로 켜져 있다(`SMAppService`). 설정에서 끌 수 있다.
- 업데이트(Sparkle)는 백그라운드에서 매시간 확인한다. 체인이 더 새 프로토콜을 예약하면 바로 확인한다.
- 결제 링크 `aether://pay?to=0x…&amount=1.5&memo=…&callback=https://…`: 웹 페이지가 확장 없이 결제를 요청한다. 앱은 송금 화면을 채워 보여 주고, 사람이 Touch ID로 승인해야 보낸다. 저절로 보내지 않는다. 결과(`tx`, `status`)는 콜백이 https일 때만 그 주소로 돌려준다. 빠진 값이 있는 링크는 무시한다.

### AI 비서 지갑의 소유자 통제 (2026-09-30)

- `aether-agent init`은 키만 만든다. 소유자가 이름과 주소를 지정해 `payee add`를 Touch ID로 승인하기 전에는 결제 세션을 만들지 않는다. 첫 세션의 기본값은 1회 1 DBLN, 24시간 10 DBLN, 7일 만료다. `policy renew`도 Touch ID를 요구한다. `--allow anyone`은 경고를 동반한 명시적 선택이다.
- Mac 지갑 보안 화면의 **비서 멈추기**와 `aether-agent stop`은 소유자 서명으로 세션을 온체인에서 제거한다. 이미 제출된 거래는 취소하지 못한다. 미승인 수취인은 결제 없이 로컬 승인 요청과 알림을 남기며, 지갑에서 이름을 입력하고 Touch ID로 추가한다.
- 새 제네시스(`node_rewards` 또는 `history_v2`)의 계정 코드만 소유자가 지정한 ERC-20 `transfer(address,uint256)`을 허용한다. 토큰마다 최소 단위로 1회·24시간 한도를 따로 둔다. 다른 셀렉터·추가 데이터·미등록 토큰은 거부한다. 테스트넷 7780에는 기존 계정 바이트코드를 그대로 사용한다.
- 세션 교체(`policy set`·`policy renew`·`payee add`)는 새 세션 ID를 부여하므로 이전 토큰 허용 목록도 폐기된다. 계속 쓸 토큰은 소유자가 다시 Touch ID로 허용한다.
- `비서 사용 내역`은 체인 영수증의 최종 성공/실패를 확인한 뒤 기록한다. 목적과 수취인 이름은 로컬의 에이전트·소유자 입력이며 체인이 증명하는 구매 사실이 아니다. 거래 해시로 영수증을 다시 확인할 수 있다. 속은 비서는 허용된 수취인에게 한도 안의 금액을 쓸 수 있다.

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

- 보내기 화면에 자산 고르기(네이티브 코인 + 보유 토큰, 항상 "SYMBOL · 0x8a9B…F41c" 표기 — 심볼만 표시하지 않는다). 자산 시트에서 토큰을 누르면 받는 사람 없이 보내기가 열린다.
- 금액은 십진 문자열을 정확히 파싱해 기본 단위로 바꾼다(부동소수점 없음, 토큰 decimals 초과 거부). calldata는 `transfer(address,uint256)` 셀렉터 `0xa9059cbb` + 단어 2개. 최대 버튼은 전체 자릿수로 채운다.
- 전송은 `prepare_call`(가스 한도 100,000; FFI에 estimateGas가 없어 고정)로 서명 — 네이티브 코인과 같은 Touch ID, 같은 활동 기록("Sent 5 NEB to 0x12…ab"), 완결 후 잔액 새로 읽기.
- 확장은 같은 calldata를 기존 wasm `prepareTx` 경로로 보낸다(노드에서 잔액을 한 번 더 확인한 뒤).

### 표시 정책(제로 트러스트 수신함)

- 메인 자산 목록: 네이티브 코인 + 이 지갑의 **서명된 트랜잭션이 건드린** 토큰(보내기·승인·호출) + 공식 목록(token-sources.json seed + waeth) + 사용자가 직접 올린 토큰.
- 남이 보내기만 한 토큰은 접힌 "미확인(Unverified)" 섹션으로, 어떤 합계에도 들어가지 않는다.
- 런치패드 출처 토큰은 "Launchpad · unverified" 배지(카탈로그가 출처를 기억, 런치패드 > dex > pool > seed 우선). 공식 토큰 심볼·이름과 같거나 비슷하면(접기·편집거리 1) 사칭 경고.
- 가격·수익률은 어디에도 없다.

### 보내기 흐름 보호

- (a) 주소 오염: 받는 사람이 과거 송금 주소와 **앞 4 + 뒤 4 글자만** 같고 다른 주소면 차단 경고 — 전체 주소를 비교했다는 확인 전까지 보내기 버튼이 잠긴다.
- (b) 처음 보내는 주소에는 가벼운 안내.
- (c) 드라이런: 서명 전에 같은 전송을 보내는 사람 주소로 `eth_call`(상태 변경 없음). revert면 이유(Error(string) 디코딩)를 보여 주고 보내지 않는다. 앱은 로컬 노드 루프백(127.0.0.1:18545)으로 호출(FFI `eth_call`은 from을 못 줘서 토큰 전송이 무조건 revert한다); 노드가 없으면 네이티브 코인은 FFI `eth_call`으로 수신 불가 컨트랙트만 검사하고, 토큰은 "확인 못 함"으로 솔직하게 통과. 확장은 전체 드라이런.

### 저장 규칙

파생 집합("내가 보낸 주소", "내 트랜잭션이 건드린 토큰")은 지갑이 이미 추적하던 활동/영수증에서 계산한다(체인에 아무것도 새로 쓰지 않는다). 기기에만 저장하는 것은 사용자 선택(숨김·표시, UserDefaults `tokenChoices.<chain>` / `chrome.storage.local`)이다. 드라이런 결과는 저장하지 않는다. 도우미 상단 주석과 앱 보안 화면에 이 문장을 뒀다: "이 검사는 이 기기의 공개 체인 데이터와 설정만 읽는다. 체인에는 아무것도 새로 쓰지 않는다."

### 주소별 확정 내역과 연결 지갑

- 노드는 확정 블록의 송신자·네이티브 코인 수신자·ERC-20 `Transfer` 로그의 송수신자·노드/증명 보상 수령자를 주소별로 색인한다. 위임 계정의 `execute` 배치도 성공한 내부 송금의 모든 수신자를 색인하며, 같은 거래에서 같은 주소로 여러 번 보냈다면 금액을 합쳐 한 행으로 둔다. `aether_accountHistory [address, before_cursor?, limit]`는 최신순으로 돌려주며 `history_start`와 `indexed_height`를 함께 보낸다. 기존 저장소를 업그레이드한 노드는 색인을 시작한 높이부터 표시한다. 가지치기한 블록의 색인도 함께 지운다.
- 앱은 FFI의 읽기 분산 경로로 이 데이터를 읽고, 로컬 대기 거래와 해시·주소 쌍으로 합친다. 한 거래가 연결 지갑 두 주소를 건드리면 두 활동 항목을 모두 보여 준다. 상세 정보에는 **"From the node"**를 표시한다. 이 내역은 노드의 표시 데이터이며 네이티브 코인 잔액은 계속 확정 인증서와 상태 증명으로 검증한다.
- DEX Router·런치패드·TokenFactory는 배포 주소와 메서드 셀렉터가 함께 맞을 때 스왑·유동성·출시로 해석한다. 스왑은 Pair `Swap` 로그가 있어야 확정 스왑으로 표시하고, 수량은 트랜잭션 값과 `Transfer`/WAETH `Withdrawal` 로그에서 읽는다. ERC-20 토큰은 심볼만 쓰지 않고 주소를 곁들인다. 알 수 없는 호출도 메서드와 `Transfer` 로그의 토큰 이동을 보여 준다. 연결 지갑은 공개 주소만 보관하고, 서명 키나 권한은 합치지 않는다.
- 새 수신 거래 알림은 주소별 마지막 처리 높이와 해시로 한 번만 보낸다. 검증된 잔액이 인접한 블록에서 증가했는데 수신 내역이 없으면 그 높이에 잔액 증가 항목을 표시한다. 내부 네이티브 코인 이동은 별도 로그가 없으므로 이 대체 항목은 관측한 높이에 한정된다.
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

## 인앱 브라우저(Explore 탭) — 구현됨 (2026-10-05)

창업자(2026-10-05): "지갑에 브라우저가 있어야 하는 것 아닌가?" — 공개 HTTPS 탐색기는 사용자의 로컬 노드를 못 읽는다(Chrome 로컬 네트워크 접근 프롬프트, Safari 혼합 콘텐츠 — [docs/research/public-read-access-2026-10-05.md](../research/public-read-access-2026-10-05.md) §3.4). iOS에는 확장이 없고, dApp(이름 서비스, 이후 DEX·런치패드)에는 사이너가 필요하다. `Sources/ExploreTab.swift`·`BrowserController.swift`.

**탭 구성.** 사이드바(macOS)·하단 탭(iOS)에 Explore가 붙는다. 홈은 큐레이티드 항목 두 개 — 블록 탐색기(번들)와 프로젝트 사이트. 주소창은 스킴 없는 입력을 `https://` 호스트로 취급한다.

**번들 탐색기.** `apps/explorer`를 앱 리소스로 복사해(postBuildScript `Bundle explorer`, test/·package.json·README.md 제외) `eastsea-page://` 개인 스킵으로 서빙한다(`BundledPageScheme`). `file://`에 문을 열지 않고, 경로 탐색(`..`)은 거부하며(`BundledPagePath`), 모든 응답에 CSP가 붙는다 — `default-src 'none'; script-src 'self'; …; connect-src 'self' http://127.0.0.1:18545 http://127.0.0.1:18546`. 탐색기의 노드 엔드포인트(`localStorage` `aether-explorer.node`)는 매 로드마다 이 앱의 노드 포트(통상 18545, 개발망 18546)로 지정된다: 노드가 임의 오리진을 허용하므로 페이지가 직접 fetch한다.

**프로바이더.** `Resources/provider.js`를 `WKUserScript`(documentStart, 메인 프레임만)로 주입한다 — 확장의 `apps/extension/src/inpage.js` 표면을 WebKit 브리지(`WKScriptMessageHandlerWithReply`, 이름 `aether`)로 옮긴 것이고, `window.ethereum`은 takeover하지 않는다(동해 계정은 P-256). **메서드 집합은 확장의 것과 정확히 같다**(`apps/extension/src/lib/methods.js`의 READ ∪ ACCOUNT ∪ SEND + background가 직접 답하는 `eth_chainId`·`wallet_disconnect`·`aether_disconnect`): `personal_sign`·`eth_signTypedData_v4`·`wallet_switchEthereumChain`·`eth_getTransactionReceipt`는 확장이 답하지 않으므로 여기서도 **4200으로 거부**한다(receipt는 `aether_getReceipt`가 담당). 집합 일치는 `apps/extension/test/wallet-provider.test.mjs`가 지킨다. 오리진별 미응답 요청 상한도 확장과 같은 3개(-32002).

**읽기 라우팅.** 검증 경로(FFI)가 노드 응답 형태를 그대로 재현하는 것만 FFI로 답한다 — `eth_blockNumber`·`eth_getBalance`·`net_version`·`aether_accountHistory`, 그리고 `from` 없는 `eth_call`. 나머지(`eth_estimateGas`·`eth_gasPrice`·`eth_getCode`·`eth_getLogs`·`eth_getStorageAt`·`eth_getTransactionCount`, `from` 있는 `eth_call`, `aether_getAccount`, 그리고 형태가 더 풍부한 `aether_status`·`aether_getReceipt`)는 이 맥의 노드 `http://127.0.0.1:<port>`에 그대로 전달하며 "미검증"으로 표시된다.

**네이티브 검증 브리지(`window.eastsea.verify`).** 같은 `provider.js`가 `window.eastsea.verify`를 띄운다 — `block(height)`·`account(address)`·`receipt(txHash)` 세 가지 물음에 `{verified, height?, reason}`으로 답하는 표면으로, WebKit 브리지의 두 번째 핸들러(이름 `eastsea`, `WKScriptMessageHandlerWithReply`)를 지난다(`BrowserController.handleVerifyMessage`, 순수 로직은 `VerifyBridge.swift`). 답은 러스트 FFI 검증 경로에서 온다: `account`는 `verified_account_at`(위원회 증명이 담보하는 상태 읽기), `block`은 커밋티 증명서로 블록 하나의 역사를 확인하는 `verified_block`(앵커와 같은 정체성·체인·증명서 검사에서, 상태 전용 규칙 — 10분 신선도·재생 방지 바닥 — 만 뺀 것: 역사는 오래된 증명서도 역사다), `receipt`는 오늘날 어떤 블록도 영수증에 커밋하지 않으므로(`crates/light` 블록 Payload에 영수증 루트가 없다) `{verified: false, reason: "not committed"}` 한 가지뿐이다. 누가 물을 수 있는지: 번들 페이지(`eastsea-page://`)는 항상, 외부 페이지는 `https`이고 연결된 오리진일 때만(`VerifyBridge.allows`) — 연결 시트를 한 번도 통과하지 않은 페이지엔 검증자가 주어지지 않는다. 잠금 규칙은 프로바이더와 같다(잠기면 전부 4100). 브라우저 wasm이 증명서 하나에 ≈11.7 ms 걸리는 데 네이티브는 ≈0.68 ms다([docs/research/wasm-speed-2026-10-05.md](../research/wasm-speed-2026-10-05.md)) — 그래서 앱 안에서는 wasm을 내리지 않고 이 브리지로 답한다. 탐색기는 하나의 코드 경로에서 기능 감지로 이 표면을 쓴다(`apps/explorer/js/verify.js`): 앱 안이면 `window.eastsea.verify`, 밖이면 확장이 쓰는 wasm 모듈(`aether_wasm.js` `verifyAccount`)을 불러 검증하고, 둘 다 없으면 "not verified" — "verified by committee certificate" 배지는 검증된 결과에만 붙는다.

**서명 요청.** `eth_requestAccounts`(연결)와 `eth_sendTransaction`(전송)은 항상 네이티브 확인 시트를 연다 — 오리진, `CallDescribe`의 액션 문장, 받는 주소, 정확한 금액·수수료, 캘리데이터. 트랜잭션 정규화는 확장의 `normalizeTx`를 그대로 미러했다(`PageTransaction.parse`: value null → 0, gas null → 0, 16진·십진 문자열·safe 정수, `to` 없으면 생성 규칙, 가스 상한 10,000,000). 일반 전송은 전송 시트와 같은 FeeChanged 흐름을 지난다 — 시트가 보여준 수수료 상한(`shownFeeWei`)이 서명 시점 견적보다 낙찰되면 FFI가 거부하고, 시트는 새 최대치를 다시 묻는다. 자동 승인은 없다. 승인하면 해시가 페이지로 돌아가고, 확정은 활동 피드가 따른다.

**오리진 권한.** 연결은 오리진(`scheme://host[:port]`, 기본 포트 생략)별로 저장되고(`SitePermissions`, UserDefaults `explore.sites`) 보안 페이지의 "Connected sites"에서 철회한다. 권한은 주소를 이름 짓는다 — 계정을 바꾸면 연결은 즉시 무효다. `from`이 연결된 주소가 아닌 전송은 4100으로 거부된다.

**잠금.** 열쇠가 없거나(`enclave == nil`) 열쇠 오류가 있으면(`keyError != nil`) **모든** 메서드가 4100으로 거부된다 — 읽기 포함. 확장의 vault가 잠긴 동안 아무것도 답하지 않는 것과 같다.

**탐색 정책.** 번들 페이지와 `https`만 연다. `http`(대문자 표기 포함), `file:`, `about:`, `javascript:` 등은 경고 없이 거부되고 이유가 주소창 아래 한 줄로 나온다. 처음 보는 외부 사이트는 일회성 경고를 지나야 하고(기기별 한 번, UserDefaults `explore.acknowledged`), 큐레이티드 도메인의 유사 도메인(`eeastsea.xyz`, `eastsea.xyz.evil.com`)과 punycode(`xn--`)는 경고에서 따로 강조된다 — 확장 `safety.js`의 fold·편집거리 규칙을 `BrowserOriginPolicy`가 미러한다. 새 창·팝업은 없고 전부 같은 탭에서 열린다.

**데이터 격리.** 외부 사이트는 호스트가 바뀔 때마다 새 `WKWebsiteDataStore.nonPersistent()` 웹뷰에서 연다 — 쿠키·저장소가 사이트끼리 만나지 않고, 탭을 닫으면 사라진다. 번들 페이지만 앱의 기본 스토어를 공유한다.
