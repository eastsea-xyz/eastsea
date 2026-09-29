# 레지스트라 키 운영 (서명 전용 Mac · Secure Enclave)

규칙은 [12-launch-plan.md](../design/12-launch-plan.md) "창업자 자산 보안"과 [14-registration.md](../design/14-registration.md) 4절에 있다. 이 문서는 **레지스트라 노드가 도는 Mac과 서명 키가 사는 Mac을 분리**해 실제로 띄우는 방법이다(분석 G4·G11, [aether-node.analysis.md](../03-analysis/aether-node.analysis.md)).

## 무엇이 바뀌나

- 예전: 레지스트라 노드가 `<data>/registrar.key`(P-256 시드 파일)로 등록·재인증 증명에 서명했다. 키가 디스크에 평문으로 있고 복사할 수 있다.
- 지금: 키는 **서명 전용 Mac의 Secure Enclave**에 있고 내보낼 수 없다. 노드는 로컬 유닉스 소켓으로 그 Mac에 서명을 부탁한다(`apps/registrar-signer`). 노드의 데이터 폴더에는 레지스트라 키가 없다.
- 파일 키는 그대로 남는다: 개발망·예행연습(`--dev-registrar`, 로컬 네트워크)은 예전과 똑같이 돈다. 바뀌는 것은 `aether run --registrar-signer <socket>`을 준 노드뿐이다.
- 위원회 서명 업그레이드가 키를 **교체**하거나 두 조각을 0으로 지워 **철회**할 수 있다(교체·철회 모두 오늘 이미 온체인에 있다).

## 조각

| 조각 | 어디 | 비고 |
|---|---|---|
| 개인 키 | 서명 Mac의 Secure Enclave | 디스크에는 핸들(`enclave.key`, 324바이트)만. 내보낼 수 없고 다른 Mac에서 못 연다 |
| 도우미 | 서명 Mac, `apps/registrar-signer` | Swift CLI. `init` / `serve` / `public` / `sign` |
| 소켓 | `~/Library/Application Support/Aether/registrar/signer.sock` | 디렉터리 0700, 소켓 0600, 같은 uid만 |
| 노드 | 레지스트라 노드 | `--registrar-signer <socket>`. `RegistrarSigner` 추상화(`crates/node/src/registrar_signer.rs`)가 파일 키와 Enclave를 바꿔 끼운다 |
| 온체인 | CommitteeRegistry 슬롯 0·1 | `attestation_message`를 SHA-256으로 서명한 (r, s)를 컨트랙트가 P256VERIFY로 검증 |

## 절차

| 단계 | 명령 | 비고 |
|---|---|---|
| 1. 빌드 | `scripts/build-registrar-signer.sh --install` | `~/.local/bin/aether-registrar-signer`. 서명 전용 Mac에는 이 바이너리만 있으면 된다 |
| 2. 키 만들기 | `aether-registrar-signer init` | **owner가 1회.** 기존 키가 있으면 거부한다. Touch ID를 원하면 `init --touch-id` |
| 3. 공개키 | `aether-registrar-signer public` | x‖y hex. `aether network --registrar <hex>`(제네시스)나 위원회 업그레이드에 쓴다 |
| 4. 서비스 | `aether-registrar-signer serve` | 포그라운드. 한 Mac에 하나만(두 번째는 "already served"로 거부). 상주 잡은 아직 없다 |
| 5. 노드 | `aether run … --devicecheck-key <p8> --registrar-signer <socket>` | 로그에 `the registrar signs with the Secure Enclave on the signing Mac` |
| 6. 확인 | 로그 + `aether-registrar-signer sign --msg <hex>` | `public`과 노드 로그의 키 앞자리가 같아야 한다 |

- 노드는 시작할 때 레지스트라 키를 온체인의 슬롯 0·1과 대조한다. 다르면 경고하고(`rotated by a committee upgrade`) 등록 RPC는 증명에 서명하지 않고 거부한다.
- `--registrar-signer`는 `--devicecheck-key`가 있어야 하고 `--dev-registrar`와 같이 못 쓴다(개발 레지스트라는 공개 dev 키로 서명한다).
- 레지스트라 서비스를 돌리지 않는 노드(검증자)는 어느 키도 읽지 않는다.

## 접근성: 왜 `AfterFirstUnlock`인가

레지스트라는 24시간 타이머로 재인증에 서명한다. 사람이 화면 앞에 없을 때가 대부분이다.

- `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`는 **화면이 잠들면** 키 생성·서명이 `-25308 errSecInteractionNotAllowed`로 실패한다(지갑은 이 오류를 "Unlock this device"로 보여준다). 2026-09-30 이 Mac에서 확인: `SecKeyCreateRandomKey` 프로브가 WhenUnlocked에서 실패, AfterFirstUnlock에서 성공. 서명 아이덴티티를 Developer ID로 바꿔도 같았다.
- 그래서 `init`(Touch ID 없이)은 `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` + `.privateKeyUsage`로 만든다: 재부팅 뒤 첫 로그인 전까지는 서명하지 못하고, 그 뒤로는 사람 없이 서명한다.
- `init --touch-id`는 `WhenUnlocked` + `.userPresence`다: 서명마다 Touch ID, 화면이 켜져 있고 잠금 해제되어 있어야 한다. 자동 서명과 함께 쓸 수 없으므로 시험·개발용이다.

## low-s 정규화 (Secure Enclave가 하지 않는 것)

- Secure Enclave는 s를 정규화하지 않는다. 같은 키로 40개를 서명해 보면 **절반 남짓이 high-s**(s > n/2)다(2026-09-30 측정: 20개 중 8개).
- 노드(`aether_crypto::verify`)와 체인은 high-s를 거부한다. 그래서 도우미가 `sign`에서 s > n/2면 `n − s`로 뒤집어 항상 low-s를 돌려준다. r‖s 64바이트, ECDSA P-256/SHA-256은 그대로다.
- 회귀 시험: `crates/node/src/registrar_signer.rs`의 `the_secure_enclave_helper_signs_attestations_the_node_accepts`(아래 "시험"의 ignored 시험).

## 소켓 프로토콜

한 연결에 JSON 한 줄 요청, JSON 한 줄 응답. 노드 쪽 구현이 계약이다(`registrar_signer.rs` 모듈 문서).

```text
→ {"op":"public"}
← {"ok":true,"public":"<x hex><y hex>","secure_enclave":true}
→ {"op":"sign","msg":"<message hex>"}
← {"ok":true,"r":"<32바이트 hex>","s":"<32바이트 hex>"}
← {"ok":false,"error":"…"}
```

- 노드는 도우미를 믿지 않는다: `connect`에서 공개키가 P-256 위의 점인지 확인하고, **모든 서명을 그 키로 다시 검증**한 뒤에 쓴다. 다른 키로 서명하는 도우미는 아무것도 못 한다.
- 서명은 도우미 안에서 한 번에 하나씩 직렬화된다(Enclave 연산, Touch ID 프롬프트).

## 위원회 교체·철회

업그레이드 JSON(`aether upgrade-sign`에 주는 파일)에 필드를 넣는다. **프로토콜 2 이상**이어야 하고, 일반 업그레이드는 7일(604,800블록) 예고 뒤 활성화 블록에서 적용된다. 긴급 업그레이드는 위원 전원의 별도 서명이 있을 때만 1에포크로 줄어든다.

```json
{ "chain_id": 7780, "protocol": 3, "activate_at": 12345678,
  "registrar": ["<새 x hex>", "<새 y hex>"],
  "releases": [ … ], "notes": "registrar key rotation" }
```

```bash
aether upgrade-sign --data <검증자 데이터 폴더> --network network.json --upgrade upgrade.json   # 위원마다
aether upgrade-combine --network network.json partials/*.json > upgrade.signed.json
aether upgrade-verify --network network.json --signed upgrade.signed.json
```

| 하고 싶은 것 | `registrar` | 결과 |
|---|---|---|
| 키 교체 | 새 키의 x·y | 활성화 블록에서 슬롯 0·1이 새 키가 된다. 옛 키로 만든 증명은 즉시 무효 |
| 등록 중지(철회) | 둘 다 `0x00…0` (32바이트 0) | 어떤 증명도 검증되지 않는다. **이미 등록한 후보는 그대로 남는다**(위원회 투표 대상 유지) |

- 회전 뒤 서명 Mac은 `init`으로 새 키를 만들어 `public`을 위원회에 넘기고, 발효 블록 뒤에 노드를 새 소켓으로 다시 띄운다. 노드는 발효 전(예고 기간)에는 옛 키로 계속 서명하고, 발효 뒤에는 경고 + 등록 RPC 거부로 멈춘다.
- 철회는 새 키를 넣지 않는다. 되살리려면 그 뒤에 다시 키를 넣는 업그레이드를 한 번 더 올린다.
- 키를 도난당했을 때 실제로 막아 주는 것은 이 철회·교체와 온체인의 에포크당 신규 등록 상한(16건, 프로토콜 2), 최소 24에포크 연속 참여, 한 번에 1/3 미만 교체다([14-registration.md](../design/14-registration.md) 4절).

## 서명 Mac을 잃으면

- **`enclave.key`를 백업해도 소용없다.** 다른 Mac에서는 열리지 않는다. 이 키의 "백업"은 위원회 교체 절차다.
- 새 Mac에서 `init` → `public` → 위원회 교체 업그레이드(7일). 그동안 옛 키는 살아 있으므로, 그 키가 남의 손에 있을 수 있다면 먼저 `registrar`를 0으로 두는 철회를 올린다(위원 전원 서명이 있으면 1에포크).
- 키가 사라진 채로 두면 다음 재인증부터 새 등록이 멈춘다. 노드 로그의 `registrar:` 경고가 그 신호다.

## 시험

| 무엇 | 명령 |
|---|---|
| 추상화·프로토콜·거짓 도우미 거부 | `cargo test -p aether-node --lib registrar_signer` |
| **이 Mac의 Secure Enclave가 서명한 것을 노드가 받는다**(ignored) | `AETHER_REGISTRAR_HOME=<home> cargo test -p aether-node --lib registrar_signer -- --ignored` |
| 위원회 업그레이드의 철회가 활성화 블록에서 적용 | `cargo test -p aether-node --test activation a_committee_upgrade_that_zeroes_the_registrar_stops_it` |
| 회전: 옛 증명은 죽고 새 키는 산다 | `cargo test -p aether-execution --test registry a_rotated_registrar_signs_and_a_revoked_one_goes_silent` |
| 파일 키가 개발망에서 그대로 | `cargo test -p aether-node --lib devicecheck` |

ignored 시험은 `scripts/build-registrar-signer.sh`로 만든 바이너리(`target/registrar-signer/aether-registrar-signer`, `AETHER_REGISTRAR_SIGNER_BIN`으로 바꿀 수 있다)와 `AETHER_REGISTRAR_HOME`의 키를 쓴다. Secure Enclave가 없는 기계(CI)에서는 돌지 않는다.

## 남은 일

- 노드는 발효된 교체를 **자동으로 따라가지 않는다**: 서명 Mac에서 새 키로 `serve`를 다시 띄우고 노드를 재시작해야 한다. 자동 재연결은 남은 일.
- 도우미를 상주시키는 launchd 잡(로그인 시 자동 시작, `caffeinate`)은 아직 없다. 지금은 운영자가 직접 띄운다. `--touch-id` 모드는 화면 잠금 상태에서 서명하지 못하므로 상주용으로는 `AfterFirstUnlock`(기본)을 쓴다.
- 앱(지갑)은 아직 이 키를 만들거나 보여 주지 않는다: `init`·`serve`는 CLI다.
